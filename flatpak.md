# GitComet Flatpak Verification and Flathub Submission

This document covers two things:

1. How to manually verify that the GitComet Flatpak installs and works.
2. How to submit GitComet to Flathub for the first time, then switch to the repo's automated update flow.

The Flatpak app ID used in this repo is `dev.gitcomet.GitComet`.

## 1. What to verify before submission

Before opening a Flathub submission, verify all of these on a Linux machine:

- the Flatpak builds successfully from this repo
- the single-file `.flatpak` bundle installs cleanly
- the app launches from the desktop and from `flatpak run`
- the app can open a real host repository
- normal Git operations work inside the sandbox
- the desktop file, icon, and metainfo look correct
- the declared permissions match what the app really needs

If you develop on macOS or Windows, do this section in a Linux VM.

## 2. Prerequisites

Install Flatpak and `flatpak-builder` from your distro, then make sure the Flathub remote exists:

```bash
flatpak remote-add --if-not-exists --user flathub https://dl.flathub.org/repo/flathub.flatpakrepo
```

Install the Flathub-maintained builder runtime used by Flathub's docs and linter:

```bash
flatpak install --user -y flathub org.flatpak.Builder
```

## 3. Clean out any previous local test install

If you already installed an older GitComet Flatpak locally, remove it first:

```bash
flatpak uninstall --user -y dev.gitcomet.GitComet || true
rm -rf builddir repo dist/dev.gitcomet.GitComet.flatpak flatpak/cargo
```

## 4. Build the local Flatpak from this repo

From the repository root:

```bash
bash scripts/prepare-flatpak-local-cargo.sh

flatpak-builder \
  --force-clean \
  --user \
  --install-deps-from=flathub \
  --repo=repo \
  --install \
  builddir \
  flatpak/dev.gitcomet.GitComet.local.yaml
```

What this does:

- vendors the current Cargo dependency graph, including git dependencies, into `flatpak/cargo`
- builds the Flatpak from `flatpak/dev.gitcomet.GitComet.local.yaml`
- exports the result to a local OSTree repo at `./repo`
- installs the Flatpak into your user Flatpak installation

## 5. Build the single-file `.flatpak` bundle

This is the install format you should manually verify before submission:

```bash
mkdir -p dist
flatpak build-bundle \
  repo \
  dist/dev.gitcomet.GitComet.flatpak \
  dev.gitcomet.GitComet \
  --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo
```

## 6. Reinstall from the bundle

First remove the locally installed build, then install the actual bundle file:

```bash
flatpak uninstall --user -y dev.gitcomet.GitComet || true
flatpak install --user -y dist/dev.gitcomet.GitComet.flatpak
```

Confirm the installed app ID:

```bash
flatpak info dev.gitcomet.GitComet
```

Inspect the shipped permissions:

```bash
flatpak info --show-permissions dev.gitcomet.GitComet
```

Expected defaults from this repo:

- `--filesystem=host`
- `--share=network`
- `--socket=ssh-auth`
- `--socket=gpg-agent`
- `--socket=wayland`
- `--socket=fallback-x11`
- `--talk-name=org.freedesktop.Flatpak`
- `--talk-name=org.freedesktop.FileManager1`
- `--talk-name=org.freedesktop.Notifications`

## 7. Launch tests

Launch from the terminal:

```bash
flatpak run dev.gitcomet.GitComet
```

Also verify that it appears correctly in the desktop launcher:

- app name is `GitComet`
- icon is present
- the app opens normally from the graphical launcher

## 8. Functional tests inside the sandbox

Use a disposable test repo so you can verify normal flows end to end.

Create one:

```bash
mkdir -p /tmp/gitcomet-flatpak-test
cd /tmp/gitcomet-flatpak-test
git init
printf '# Flatpak test\n' > README.md
git add README.md
git commit -m "Initial commit"
```

Then verify this checklist manually in the Flatpak build:

1. Open `/tmp/gitcomet-flatpak-test` in GitComet.
2. Confirm the commit history appears.
3. Edit `README.md`, then stage and unstage the change.
4. Create a commit from inside GitComet.
5. If you use SSH remotes, add a disposable SSH remote and verify fetch/pull/push.
6. If you sign commits or tags, verify GPG agent access.
7. If you rely on difftool or mergetool paths, test at least one real conflict or diff flow.

Important things to watch for:

- no missing repository access errors
- no missing `git` binary errors
- auth prompts work
- the app is using the host Git successfully
- temp-file-based auth, merge, or diff flows work

## 9. Lint and metadata checks

Run the same basic checks Flathub expects before you submit:

```bash
flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest flatpak/dev.gitcomet.GitComet.local.yaml
flatpak run --command=flatpak-builder-lint org.flatpak.Builder repo repo
flatpak run --command=flatpak-builder-lint org.flatpak.Builder appstream flatpak/dev.gitcomet.GitComet.metainfo.xml
desktop-file-validate assets/linux/dev.gitcomet.GitComet.desktop
appstreamcli validate --no-net flatpak/dev.gitcomet.GitComet.metainfo.xml
```

Do not submit until these are clean or you explicitly know which exception you need.

## 10. Prepare the first public release assets

For the first Flathub submission, you need public release assets that Flathub can fetch.

The minimum public release assets needed for Flathub are:

- `gitcomet-v<VERSION>-source.tar.gz`
- `gitcomet-v<VERSION>-cargo-vendor.tar.gz`
- `dev.gitcomet.GitComet.yaml`
- `flathub.json`

### Recommended first-release path

Use a normal public GitHub release first, then use this repo's existing build workflow to attach the Flatpak/Flathub assets.

Example for version `0.2.0`:

```bash
git tag -a v0.2.0 -m "GitComet v0.2.0"
git push origin v0.2.0
gh release create v0.2.0 \
  --repo Auto-Explore/GitComet \
  --title "GitComet v0.2.0" \
  --generate-notes
RELEASE_ID="$(gh release view v0.2.0 --repo Auto-Explore/GitComet --json id --jq '.id')"
gh workflow run build-release-artifacts.yml \
  -f tag=v0.2.0 \
  -f version=0.2.0 \
  -f release_id="$RELEASE_ID"
```

After that workflow finishes, the GitHub release should contain:

- the Linux Flatpak bundle
- the source tarball
- the vendored Cargo tarball
- the rendered Flathub manifest
- `flathub.json`

Note:

- for the very first Flathub submission, do not rely on `release-manual-main.yml` unless the Flathub app repo already exists and `FLATHUB_TOKEN` has been configured
- the app-specific Flathub repo does not exist until the initial submission is accepted

## 11. Download the submission files

Download these release assets locally:

- `dev.gitcomet.GitComet.yaml`
- `flathub.json`

You will put those into the Flathub submission PR.

Before you open the PR, lint the exact rendered manifest you are about to submit:

```bash
flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest dev.gitcomet.GitComet.yaml
```

## 12. Open the first Flathub submission PR

New Flathub app submissions go through `flathub/flathub` and must target the `new-pr` base branch.

### Option A: with GitHub CLI

```bash
gh repo fork --clone flathub/flathub
cd flathub
git checkout --track origin/new-pr
git checkout -b gitcomet-submission
```

### Option B: manual Git setup

1. Fork `flathub/flathub` on GitHub.
2. Make sure "Copy the master branch only" is unchecked when you fork.
3. Clone your fork's `new-pr` branch:

```bash
git clone --branch=new-pr git@github.com:YOUR_GITHUB_USERNAME/flathub.git
cd flathub
git checkout -b gitcomet-submission
```

### Add the GitComet submission files

Copy these into the root of the cloned `flathub` repo:

- `dev.gitcomet.GitComet.yaml`
- `flathub.json`

Then commit and push:

```bash
git add dev.gitcomet.GitComet.yaml flathub.json
git commit -m "Add dev.gitcomet.GitComet"
git push origin gitcomet-submission
```

### Create the pull request

Open a PR with:

- base repo: `flathub/flathub`
- base branch: `new-pr`
- title: `Add dev.gitcomet.GitComet`

Do not target `master` or the default branch.

If you use GitHub CLI, this is the equivalent command:

```bash
gh pr create \
  --repo flathub/flathub \
  --base new-pr \
  --head YOUR_GITHUB_USERNAME:gitcomet-submission \
  --title "Add dev.gitcomet.GitComet"
```

## 13. Handle review

While the submission is under review:

- keep using the same PR
- push fixes to the same branch
- do not close and reopen the PR just to address comments
- do not merge the default Flathub branch into your submission branch

If reviewers ask for permission changes, metadata fixes, or linter exceptions, update the same PR.

## 14. Verify ownership of `gitcomet.dev`

Once the submission is accepted and you have collaborator access to the app repository, verify the app in the Flathub Developer Portal.

The verification path for this app ID is:

```text
https://gitcomet.dev/.well-known/org.flathub.VerifiedApps.txt
```

The process is:

1. Log in to Flathub.
2. Open the Developer Portal.
3. Open the GitComet app entry.
4. Open `Verification`.
5. Copy the generated token.
6. Publish that token at `https://gitcomet.dev/.well-known/org.flathub.VerifiedApps.txt`.
7. Retry verification in the portal.

If multiple apps are ever verified under `gitcomet.dev`, put each token on its own line.

## 15. Switch to automated updates after the first acceptance

After the first submission is accepted, Flathub will create an app-specific packaging repo, typically:

```text
flathub/dev.gitcomet.GitComet
```

At that point, configure these in the GitComet GitHub repo:

- secret `FLATHUB_TOKEN`
- optional variable `FLATHUB_REPO=flathub/dev.gitcomet.GitComet`
- optional variable `FLATHUB_BRANCH`
- optional variable `FLATHUB_MODE=pull_request`

Recommended setting:

- use `FLATHUB_MODE=pull_request` first
- only switch to `push` if you intentionally want direct commits to the Flathub app repo

This repo already has a release-time Flathub deployment workflow. Once the app repo exists and the token is configured, `release-manual-main.yml` can:

1. publish the GitHub release
2. upload the Flatpak and Flathub assets
3. open or update the Flathub packaging PR automatically

## 16. Why `flathub.json` is included

This repo ships a `flathub.json` with:

```json
{
  "disable-external-data-checker": true
}
```

That is intentional.

GitComet's GitHub release flow already creates the Flathub update payload. Disabling Flathub's default hourly external-data-checker avoids duplicate update PRs.

## 17. Quick checklist

Before first submission:

- local Flatpak build passes
- bundle install passes
- app launches and works
- linter checks pass
- public GitHub release exists
- release contains `dev.gitcomet.GitComet.yaml`
- release contains `gitcomet-v<VERSION>-cargo-vendor.tar.gz`
- release contains `flathub.json`
- submission PR targets `flathub/flathub:new-pr`

After acceptance:

- app verified with `gitcomet.dev`
- collaborator access to the app repo confirmed
- `FLATHUB_TOKEN` configured in GitHub
- `release-manual-main.yml` used for ongoing releases
