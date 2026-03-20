## GitComet

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
