## <img alt="GitComet logo" src="assets/gitcomet_logo.svg" width="26" /> GitComet 

[![Build Status](https://github.com/Auto-Explore/GitComet/actions/workflows/rust.yml/badge.svg?branch=main)](https://github.com/Auto-Explore/GitComet/actions/workflows/rust.yml)
[![Discord](https://img.shields.io/badge/Discord-Join%20chat-5865F2?logo=discord&logoColor=white)](https://discord.gg/2ufDGP8RnA)
[![Website](https://img.shields.io/badge/Website-gitcomet.dev-0A66C2?logo=googlechrome&logoColor=white)](https://gitcomet.dev)
[![AutoExplore](https://img.shields.io/badge/AutoExplore-autoexplore.ai-0B7A75?logo=safari&logoColor=white)](https://autoexplore.ai)
[![license](https://img.shields.io/github/license/Auto-Explore/gitcomet.svg)](LICENSE)
[![latest](https://img.shields.io/github/v/release/Auto-Explore/gitcomet.svg)](https://github.com/Auto-Explore/gitcomet/releases/latest)
[![downloads](https://img.shields.io/github/downloads/Auto-Explore/gitcomet/total)](https://github.com/Auto-Explore/gitcomet/releases)

**Fastest Open Source Git GUI**

GitComet is built for teams that want fast Git operations with local-first privacy, familiar workflows, and open source freedom.

Available for Linux, Windows, and macOS.

<img alt="GitComet demo" src="assets/gitcomet.gif"/>

### Download

Download the latest prebuilt binaries/installers from [GitHub Releases](https://github.com/Auto-Explore/GitComet/releases).

#### Homebrew (macOs / Linux)

install app from tap (recommended):

```bash
brew tap auto-explore/gitcomet
brew install --cask gitcomet
```


**MacOS Troubleshooting:** If you downloaded the app manually and macOS reports that GitComet *"is damaged and can't be opened"*, this is due to Apple's Gatekeeper quarantine for apps downloaded outside the App Store. To resolve this, run the following command in your terminal to remove the quarantine attribute:

```bash
xattr -d com.apple.quarantine /Applications/GitComet.app
```

optional CLI install:

```bash
brew install gitcomet-cli
```

#### AUR (Arch Linux)

```bash
git clone https://aur.archlinux.org/gitcomet.git
cd gitcomet && makepkg -si
```

#### GURU (Gentoo Linux)

```bash
emerge --ask dev-vcs/gitcomet
```

#### apt (Debian/Ubuntu)

```bash
curl -fsSL https://apt.gitcomet.dev/gitcomet-archive-keyring.gpg | sudo tee /usr/share/keyrings/gitcomet-archive-keyring.gpg >/dev/null
curl -fsSL https://apt.gitcomet.dev/gitcomet.sources | sudo tee /etc/apt/sources.list.d/gitcomet.sources >/dev/null
sudo apt update
sudo apt install gitcomet
```

### Requirements

GitComet requires a local Git installation of `2.50` or newer.

### Fast, Free, Familiar

- **Fast**: Built end-to-end in Rust for speed and efficiency using [smol](https://github.com/smol-rs/smol), [gix](https://github.com/GitoxideLabs/gitoxide), and [gpui](https://www.gpui.rs/).
- **Free**: Free to use for individuals and organizations.
- **Familiar**: Familiar history browsing and drop-in `git difftool` and `git mergetool` compatibility.

### Why GitComet

GitComet started from frustration with existing tools on huge codebases like Chromium. We could not find a product that stays responsive and functional when browsing large repositories and file diffs.

#### Chromium benchmark snapshot

| Tool | Version | Time to open Chromium repo | Memory while opening |
| --- | --- | ---: | ---: |
| GitComet | v0.2.0 | 1s | 265MB |
| GitFiend | v0.45.3 | 1s | 289MB |
| SourceGit | v2026.6 | 3.5s | 301MB |
| SmartGit | v25.1.110 | 18s | 4.8GB |
| GitKraken | v11.10.0 | 25s | 2GB |
| Megit | v0.10.0 | 29s | 14.4GB |
| Gittyup | v2.0.0 | 43s | 2.5GB + 1.5GB indexer |

Measured on Linux 6.19.6-zen (x64), Ryzen 5950x, 128GB DDR4. Detailed test steps will be published.

### Editions (planned)

#### Open Source

- **Price**: €0 forever
- **Usage**: Free for personal and commercial use
- **Includes**:
  - Full local-first desktop workflow
  - Git remotes, pull/push, staging, commits
  - Worktrees, branching, and full history
  - Multi-repository browsing
  - Inline and side-by-side diffs
  - 2-way and 3-way merge tools

#### Professional

- **Price**: €20 lifetime access (limited-time early adopter offer)
- **Includes everything in Open Source, plus**:
  - Claude Code, Codex, and GitHub CLI integrations
  - Code test coverage workflows
  - GitHub and Azure DevOps integrations
  - Priority improvements during early access
- Join waitlist: [gitcomet.dev/#pricing](https://gitcomet.dev/#pricing)

### Build from source

```bash
cargo build -p gitcomet --features ui-gpui,gix
cargo run -p gitcomet --features ui-gpui,gix -- /path/to/repo
```

### Contributing

Developer setup, workspace layout, testing, and coverage docs live in `CONTRIBUTING.md`.

### Using as a Git difftool / mergetool

GitComet can be used as a standalone diff and merge tool invoked by `git difftool` and `git mergetool`. It supports both headless (algorithm-only) and GUI (interactive GPUI window) modes.

#### Setup / uninstall (recommended)

```bash
# Configure Git globally to use GitComet for both difftool + mergetool
gitcomet setup

# Remove GitComet integration safely
gitcomet uninstall
```

- Use `--local` to target only the current repository instead of global config.
- Use `--dry-run` to print the commands before applying changes.

This setup registers both headless and GUI variants with `guiDefault=auto`, so Git chooses GUI when display is available and falls back to headless otherwise.
`setup`/`uninstall` are designed to be idempotent.

<details>
<summary>Show detailed setup/uninstall behavior and manual commands</summary>

Built-in `setup` writes these Git config entries:

```bash
GITCOMET_BIN="/absolute/path/to/gitcomet"

# Headless tool: algorithm-only merge/diff for CI, scripts, and no-display environments
git config --global merge.tool gitcomet
git config --global mergetool.gitcomet.cmd \
  "'$GITCOMET_BIN' mergetool --base \"\$BASE\" --local \"\$LOCAL\" --remote \"\$REMOTE\" --merged \"\$MERGED\""
git config --global mergetool.trustExitCode true
git config --global mergetool.gitcomet.trustExitCode true
git config --global mergetool.prompt false

git config --global diff.tool gitcomet
git config --global difftool.gitcomet.cmd \
  "'$GITCOMET_BIN' difftool --local \"\$LOCAL\" --remote \"\$REMOTE\" --path \"\$MERGED\""
git config --global difftool.trustExitCode true
git config --global difftool.gitcomet.trustExitCode true
git config --global difftool.prompt false

# GUI tool: opens focused GPUI windows for interactive diff/merge
git config --global merge.guitool gitcomet-gui
git config --global mergetool.gitcomet-gui.cmd \
  "'$GITCOMET_BIN' mergetool --gui --base \"\$BASE\" --local \"\$LOCAL\" --remote \"\$REMOTE\" --merged \"\$MERGED\""
git config --global mergetool.gitcomet-gui.trustExitCode true

git config --global diff.guitool gitcomet-gui
git config --global difftool.gitcomet-gui.cmd \
  "'$GITCOMET_BIN' difftool --gui --local \"\$LOCAL\" --remote \"\$REMOTE\" --path \"\$MERGED\""
git config --global difftool.gitcomet-gui.trustExitCode true

# Auto-select GUI tool when DISPLAY is available, headless otherwise
git config --global mergetool.guiDefault auto
git config --global difftool.guiDefault auto
```

Built-in `setup` stores previous user values for shared generic keys under `gitcomet.backup.*` (when needed).  
Built-in `uninstall` restores those backups only when the key still has the setup-managed value. If the user changed a setting after setup, uninstall preserves that user-edited value and then removes GitComet-specific keys.

</details>

#### CLI modes

**Difftool:**

```bash
gitcomet difftool --local <path> --remote <path> [--path <display_name>] [--label-left <label>] [--label-right <label>]
```

Also reads `LOCAL`/`REMOTE` from environment as a fallback when invoked by Git.

**Mergetool:**

```bash
gitcomet mergetool --local <path> --remote <path> --merged <path> [--base <path>] [--label-local <label>] [--label-remote <label>] [--label-base <label>]
```

Also reads `LOCAL`/`REMOTE`/`MERGED`/`BASE` from environment. Base is optional for add/add conflicts.

#### Compatibility

KDiff3 and Meld invocation forms are supported (`--L1/--L2/--L3`, `-o/--output/--out`, `--base`, positional arguments), so GitComet can be a drop-in replacement.

### Themes

GitComet supports built-in themes and user-provided custom themes.

Themes are loaded from JSON bundle files. GitComet ships with built-in themes and copies them into your per-user themes directory.

GitComet loads custom themes from JSON bundle files in your per-user themes directory.

##### Theme File Location

GitComet only loads `.json` files from the user themes directory:

| Platform | Themes directory |
| --- | --- |
| Linux | `$XDG_DATA_HOME/gitcomet/themes` or `~/.local/share/gitcomet/themes` |
| macOS | `~/Library/Application Support/gitcomet/themes` |
| Windows | `%LOCALAPPDATA%\\gitcomet\\themes` or `%APPDATA%\\gitcomet\\themes` |

##### JSON Schema

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
        "keyword": "#7ec5ffff",                         // Optional
        "keyword_control": "#8fd8ffff",                 // Optional
        "number": "#9edb63ff",                          // Optional
        "boolean": "#b4e07aff",                         // Optional
        "function": "#78c4ffff",                        // Optional
        "function_method": "#87d0ffff",                 // Optional
        "function_special": "#96dbffff",                // Optional
        "type": "#ffc06aff",                            // Optional
        "type_builtin": "#ffce87ff",                    // Optional
        "type_interface": "#ffd9a3ff",                  // Optional
        "variable": "#f3f6fbff",                        // Optional
        "variable_parameter": "#c7d0deff",              // Optional
        "variable_special": "#70c5ffff",                // Optional
        "property": "#66c2ffff",                        // Optional
        "constant": "#9edb63ff",                        // Optional
        "operator": "#c5ceddff",                        // Optional
        "punctuation": "#b4beceff",                     // Optional
        "punctuation_bracket": "#c2cadaff",             // Optional
        "punctuation_delimiter": "#a9b4c7ff",           // Optional
        "string": "#ffd27aff",                          // Optional
        "string_escape": "#8ce3b4ff",                   // Optional
        "tag": "#ffc06aff",                             // Optional
        "attribute": "#74caffff",                       // Optional
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

##### Required Theme Fields

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

##### Colors Schema

###### Required color fields

`window_bg`, `surface_bg`, `surface_bg_elevated`, `active_section`, `border`, `text`, `text_muted`, `accent`, `hover`, `active`, `focus_ring`, `focus_ring_bg`, `scrollbar_thumb`, `scrollbar_thumb_hover`, `scrollbar_thumb_active`, `danger`, `warning`, `success`

###### Optional color fields

`tooltip_bg`, `tooltip_text`, `diff_add_bg`, `diff_add_text`, `diff_remove_bg`, `diff_remove_text`, `input_placeholder`, `accent_text`, `graph_lane_palette`, `graph_lane_hues`

###### Color value format

Most color fields accept either:

- a hex RGBA string such as `#0d1016ff`
- an object with `hex` plus `alpha`, for example `{ "hex": "#5ac1feff", "alpha": 0.60 }`

Use `graph_lane_palette` for an explicit list of colors, or `graph_lane_hues` for a list of hue values that GitComet turns into graph lane colors automatically.

If you omit optional diff colors, tooltip colors, placeholder color, accent text color, or syntax colors, GitComet falls back to built-in defaults.

More generally, omitted optional values are either derived from the theme's base colors or filled with built-in defaults, depending on the field.

##### Syntax Schema

The `syntax` object is optional. Supported keys are:

`comment`, `comment_doc`, `string`, `string_escape`, `keyword`, `keyword_control`, `number`, `boolean`, `function`, `function_method`, `function_special`, `type`, `type_builtin`, `type_interface`, `variable`, `variable_parameter`, `variable_special`, `property`, `constant`, `operator`, `punctuation`, `punctuation_bracket`, `punctuation_delimiter`, `tag`, `attribute`, `lifetime`

Use `type` in JSON for the main type-name color.

##### Radii Schema

The `radii` object is required and must include:

- `panel`
- `pill`
- `row`

These values are numeric and control the corner radius used by major UI elements.

##### Overrides And Validation Behavior

- GitComet loads every `.json` file in the themes directory.
- A runtime theme can override a bundled theme by reusing the same `key`.
- A file that cannot be read or parsed is ignored.
- GitComet does not expose a separate machine-readable JSON Schema file today; the implementation in `GitComet/crates/gitcomet-ui-gpui/src/theme.rs` is the source of truth.

### Crash logs

If the app crashes due to a Rust panic, GitComet writes a crash log to:

- Linux: `$XDG_STATE_HOME/gitcomet/crashes/` (fallback: `~/.local/state/gitcomet/crashes/`)
- macOS: `~/Library/Logs/gitcomet/crashes/`
- Windows: `%LOCALAPPDATA%\gitcomet\crashes\` (fallback: `%APPDATA%\gitcomet\crashes\`)

On next startup, GitComet can prompt you to report the crash as a prefilled
GitHub issue in `Auto-Explore/GitComet`, including app version, platform,
panic details, and a trimmed backtrace.

### Prior work and ideas inspired by:

SourceTree, GitKraken, Zed, GPUI, KDiff3, Meld, Github Desktop, Git, Gix, Rust, Smol, and many more.

This project has been created with the help of AI tools, including OpenAI Codex and Claude Code.

### Star History

[![Star History Chart](https://api.star-history.com/svg?repos=Auto-Explore/gitcomet&type=Date)](https://star-history.com/#Auto-Explore/gitcomet&Date)

### License

GitComet is licensed under the GNU Affero General Public License Version 3
(AGPL-3.0-only). See `LICENSE-AGPL-3.0`.

Copyright (C) 2026 AutoExplore Oy  
Contact: info@autoexplore.ai
