## GitComet

[![Build Status](https://github.com/Auto-Explore/GitComet/actions/workflows/rust.yml/badge.svg?branch=main)](https://github.com/Auto-Explore/GitComet/actions/workflows/rust.yml)

Fast, resource-efficient, fully open source Git GUI written in Rust, targeting GitKraken/SourceTree/GitHub Desktop-class workflows using `gpui` for the UI.

### Goals

- Fast UI for very large repositories
- Support `git difftool` and `git mergetool` flows
- Open source with a permissive license (AGPL-3.0)
- Cross-platform (Windows, Linux, macOS)

### Quick start

```bash
cargo build -p gitcomet-app --features ui-gpui,gix
cargo run -p gitcomet-app --features ui-gpui,gix -- /path/to/repo
```

### Contributing

Developer setup, workspace layout, testing, and coverage docs live in `CONTRIBUTING.md`.

### Using as a Git difftool / mergetool

GitComet can be used as a standalone diff and merge tool invoked by `git difftool` and `git mergetool`. It supports both headless (algorithm-only) and GUI (interactive GPUI window) modes.

#### Setup / uninstall (recommended)

```bash
# Configure Git globally to use GitComet for both difftool + mergetool
gitcomet-app setup

# Remove GitComet integration safely
gitcomet-app uninstall
```

- Use `--local` to target only the current repository instead of global config.
- Use `--dry-run` to print the commands before applying changes.

This setup registers both headless and GUI variants with `guiDefault=auto`, so Git chooses GUI when display is available and falls back to headless otherwise.
`setup`/`uninstall` are designed to be idempotent.

<details>
<summary>Show detailed setup/uninstall behavior and manual commands</summary>

Built-in `setup` writes these Git config entries:

```bash
GITCOMET_BIN="/absolute/path/to/gitcomet-app"

# Headless tool — algorithm-only merge/diff for CI, scripts, and no-display environments
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

# GUI tool — opens focused GPUI windows for interactive diff/merge
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
gitcomet-app difftool --local <path> --remote <path> [--path <display_name>] [--label-left <label>] [--label-right <label>]
```

Also reads `LOCAL`/`REMOTE` from environment as a fallback when invoked by Git.

**Mergetool:**

```bash
gitcomet-app mergetool --local <path> --remote <path> --merged <path> [--base <path>] [--label-local <label>] [--label-remote <label>] [--label-base <label>]
```

Also reads `LOCAL`/`REMOTE`/`MERGED`/`BASE` from environment. Base is optional for add/add conflicts.

#### Compatibility

KDiff3 and Meld invocation forms are supported (`--L1/--L2/--L3`, `-o/--output/--out`, `--base`, positional arguments), so GitComet can be a drop-in replacement.

### Crash logs

If the app crashes due to a Rust panic, GitComet writes a crash log to:

- Linux: `$XDG_STATE_HOME/gitcomet/crashes/` (fallback: `~/.local/state/gitcomet/crashes/`)
- macOS: `~/Library/Logs/gitcomet/crashes/`
- Windows: `%LOCALAPPDATA%\gitcomet\crashes\` (fallback: `%APPDATA%\gitcomet\crashes\`)

On next startup, GitComet can prompt you to report the crash as a prefilled
GitHub issue in `Auto-Explore/GitComet`, including app version, platform,
panic details, and a trimmed backtrace.

### License

GitComet is licensed under the GNU Affero General Public License Version 3
(AGPL-3.0-only). See `LICENSE-AGPL-3.0`.

Copyright (C) 2026 AutoExplore Oy  
Contact: info@autoexplore.ai
