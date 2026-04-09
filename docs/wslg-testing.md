# GitComet WSLg Testing

### WSLg

GitComet's Linux build can run inside WSL with WSLg.

- Launch the Linux binary from a WSL distro with `WAYLAND_DISPLAY` or `DISPLAY` available.
- Prefer repositories stored inside the distro filesystem such as `~/src/repo` instead of `/mnt/c/...`.
- If Linux desktop openers are unavailable, install `wslu` to provide `wslview` for URL and file-opening fallbacks.
- For prebuilt Linux binaries on Debian/Ubuntu/WSLg, install the GUI runtime libraries: `libxcb1`, `libxkbcommon0`, and `libxkbcommon-x11-0`.
- If GPUI panics with `UnsupportedVersion` under WSLg, force the X11 backend by unsetting `WAYLAND_DISPLAY` and `WAYLAND_SOCKET`, then export `XDG_SESSION_TYPE=x11` before launch.
- For source builds inside WSL, install the Linux UI dependencies used elsewhere in this repo: `pkg-config`, `libxcb1-dev`, `libxkbcommon-dev`, and `libxkbcommon-x11-dev`.
- GUI `git difftool` and `git mergetool` work under WSLg because `gitcomet setup` already selects the GUI tool when display environment variables are present.
- Manual validation steps for developers live in `docs/wslg-testing.md`.


This file documents how to validate GitComet's Linux GPUI application inside Windows WSL with WSLg.

Source of truth:
- `crates/gitcomet-ui-gpui/src/linux_gui_env.rs`
- `crates/gitcomet-ui-gpui/src/view/platform_open.rs`
- `crates/gitcomet-ui-gpui/src/app.rs`

Notes:
- This covers the Linux `gitcomet` binary launched from a WSL 2 distro.
- This does not cover the native Windows build.
- Prefer repositories inside the distro filesystem such as `~/src/repo`.
- Repositories under `/mnt/c/...` are best-effort only.

## Test Matrix

| Scenario | Expected result |
| --- | --- |
| Launch GitComet from a WSLg shell | Main GPUI window opens |
| Launch without GUI environment variables | Launch fails with a clear X11 / Wayland / WSLg message |
| Open a repo from the distro filesystem | Repo opens normally |
| Open a repo from `/mnt/c/...` | Best-effort behavior; note any slowdown or path issues |
| Trigger "Open Repository" | Native picker opens, or GitComet falls back to manual path entry |
| Open external URLs or file locations | Linux openers work via `xdg-open`, `gio open`, or `wslview` under WSL |
| Run `git difftool --gui` | Focused diff window opens |
| Run `git mergetool --gui` | Focused merge window opens |

## Host Setup

Run these commands in Windows PowerShell:

```powershell
wsl --install -d Ubuntu
wsl --update
wsl --shutdown
wsl -l -v
```

Expected:
- Your Linux distro is listed with `VERSION` = `2`.
- After `wsl --shutdown`, reopening the distro starts a fresh WSLg session.

## Distro Setup

Run these commands inside the WSL distro:

```bash
sudo apt update
sudo apt install -y git libxcb1 libxkbcommon0 libxkbcommon-x11-0 wslu
```

If you are building GitComet from source inside WSL, also install the development headers:

```bash
sudo apt install -y pkg-config libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev
```

If you are building from source, install the Rust toolchain normally for this repository before continuing.

## GUI Session Sanity Check

Still inside WSL, verify the session variables that GitComet uses to decide whether GPUI can launch:

```bash
echo "DISPLAY=$DISPLAY"
echo "WAYLAND_DISPLAY=$WAYLAND_DISPLAY"
echo "XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR"
```

Expected:
- `DISPLAY` may be set for X11-based sessions.
- `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR` should be set for Wayland-only WSLg sessions.
- If all three are empty, the positive launch tests should fail and the negative launch test below should match that behavior.

## Build GitComet

Use the Linux filesystem for the checkout when possible:

```bash
cd ~/src/GitComet3
cargo build -p gitcomet --features ui-gpui,gix
```

If your current checkout lives on the Windows filesystem, move or reclone it into the distro filesystem before running the WSLg tests.

The binary will be available at:

```bash
~/src/GitComet3/target/debug/gitcomet
```

## Main App Smoke Test

Create a disposable repository inside WSL and launch GitComet against it:

```bash
mkdir -p ~/src/wslg-smoke
cd ~/src/wslg-smoke
git init -b main
git config user.name "WSLg Test"
git config user.email "wslg-test@example.invalid"
printf "hello\n" > README.md
git add README.md
git commit -m "init"
~/src/GitComet3/target/debug/gitcomet ~/src/wslg-smoke
```

Verify:
- The main window opens.
- Keyboard and mouse input work.
- The repo loads without crashing.
- Closing and reopening the app from the same WSL shell continues to work.

## Repository Open Tests

From the running app:

1. Press `Ctrl-O`.
2. If the native folder picker opens, select `~/src/wslg-smoke` and confirm the repo loads.
3. If the native picker is unavailable in that WSLg session, confirm GitComet falls back to the manual repository path entry panel instead of failing silently.
4. Repeat with a repo stored under `/mnt/c/...` and note any behavioral differences.

The manual-entry fallback is part of the intended WSLg support because native pickers may not be available in every WSL desktop integration setup.

## External Opener Tests

Check which helpers are available:

```bash
which xdg-open || true
which gio || true
which wslview || true
```

From the app, trigger any action that opens a browser URL or reveals a file location.

Expected:
- `xdg-open` works when available.
- `gio open` works as a fallback when available.
- Under WSL, `wslview` from `wslu` is an acceptable fallback if the Linux desktop opener path is unavailable.

## Negative Launch Test

This validates the new launch guard and error message path.

Run GitComet with the GUI variables removed:

```bash
env -u DISPLAY -u WAYLAND_DISPLAY -u XDG_RUNTIME_DIR \
  ~/src/GitComet3/target/debug/gitcomet
```

Expected:
- GitComet exits immediately.
- The error mentions the missing GUI session and, under WSL detection, references WSLg / `WAYLAND_DISPLAY` or `DISPLAY`.

## Difftool Smoke Test

Configure the local repo to use the built GitComet binary, then open a GUI diff:

```bash
cd ~/src/wslg-smoke
~/src/GitComet3/target/debug/gitcomet setup --local
printf "second line\n" >> README.md
git difftool --gui -y HEAD -- README.md
```

Expected:
- Git starts the GitComet GUI difftool path.
- A focused diff window opens under WSLg.

## Mergetool Smoke Test

Create a simple conflict and run the GUI mergetool:

```bash
mkdir -p ~/src/wslg-merge-smoke
cd ~/src/wslg-merge-smoke
git init -b main
git config user.name "WSLg Test"
git config user.email "wslg-test@example.invalid"
printf "base\n" > conflict.txt
git add conflict.txt
git commit -m "base"
git checkout -b feature
printf "feature\n" > conflict.txt
git commit -am "feature change"
git checkout main
printf "main\n" > conflict.txt
git commit -am "main change"
~/src/GitComet3/target/debug/gitcomet setup --local
git merge feature || true
git mergetool --gui
```

Expected:
- Git detects the conflict.
- GitComet opens the focused GPUI merge window under WSLg.

## Troubleshooting

- If no window appears, rerun `wsl --update` and `wsl --shutdown` from PowerShell, then reopen the distro.
- If launch fails immediately, check `DISPLAY`, `WAYLAND_DISPLAY`, and `XDG_RUNTIME_DIR` in the WSL shell you launched from.
- If browser or file-manager actions fail, install `wslu` and verify `wslview` is present.
- If repo access is unreliable or slow, move the test repo from `/mnt/c/...` to `~/src/...`.
- If `Ctrl-O` does not show a native picker, the manual path entry fallback is the expected behavior, not a failure by itself.

## Record Results

When filing a regression or support note, include:
- Windows version
- WSL version output from `wsl -l -v`
- Distro name and version
- Values of `DISPLAY`, `WAYLAND_DISPLAY`, and `XDG_RUNTIME_DIR`
- Whether the repo lived under `~/...` or `/mnt/c/...`
- Whether `xdg-open`, `gio`, and `wslview` were present
- Which test case failed and the exact stderr output
