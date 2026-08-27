# Add GitComet to WinGet and automate releases

## Summary

GitComet already publishes signed, version-specific x64 and ARM64 WiX MSI
installers through `.github/workflows/build-release-artifacts.yml`. The
installer is machine-scoped, supports standard silent MSI operation, records
the correct publisher and version, and has a stable upgrade code. No installer
format change is expected.

WinGet publication is an upstream manifest pull request to
`microsoft/winget-pkgs`, followed by Microsoft validation and moderation. It is
not a direct package upload. The release pipeline should automate PR submission
and report its URL; availability in WinGet remains asynchronous.

Fixed decisions:

- Package ID: `AutoExplore.GitComet`
- Channel: stable releases only
- Installers: x64 and ARM64 machine-scoped MSI
- Dependency: `Git.Git` version `2.50.0` or newer
- Installer source: immutable, version-specific GitHub Release URLs
- User command: `winget install --exact --id AutoExplore.GitComet`

References:

- [Submit a manifest to the WinGet repository](https://learn.microsoft.com/en-us/windows/package-manager/package/repository)
- [WinGet community repository policies](https://github.com/microsoft/winget-pkgs/blob/master/doc/Policies.md)
- [First-time contributor checklist](https://github.com/microsoft/winget-pkgs/blob/master/doc/FirstContribution.md)
- [WingetCreate update command](https://github.com/microsoft/winget-create/blob/main/doc/update.md)
- [WingetCreate token guidance](https://github.com/microsoft/winget-create/blob/main/doc/token.md)

## Phase 1: Submit the first manifest from Windows

### 1. Confirm that GitComet is still a new package

Immediately before submission:

```powershell
winget search GitComet
```

Also search manifests and open pull requests in
[`microsoft/winget-pkgs`](https://github.com/microsoft/winget-pkgs). Do not
submit a duplicate package or duplicate version.

### 2. Prepare the submission identity

Use the same GitHub identity that will own later automated submissions.

1. Create a classic GitHub PAT with only the `public_repo` scope.
2. Do not use a fine-grained PAT; WingetCreate does not support it.
3. Do not add the optional `delete_repo` permission unless automatic cleanup of
   a failed fork is specifically wanted.
4. Expose the PAT to WingetCreate through the environment, not `--token`, so it
   is not placed on the command line:

   ```powershell
   $env:WINGET_CREATE_GITHUB_TOKEN = "<PAT>"
   ```

5. Ensure this identity completes Microsoft's contributor license agreement
   when the CLA bot requests it.

For CI, save the same credential as the GitHub Actions repository secret
`WINGET_CREATE_GITHUB_TOKEN`. Record its owner and expiry so it can be rotated
before release automation breaks.

### 3. Install or download WingetCreate

The release workflow should eventually pin WingetCreate, but the initial
interactive bootstrap can install it normally:

```powershell
winget install --exact --id Microsoft.WingetCreate
```

The planned CI pin is WingetCreate `1.12.13.0`. Its standalone executable has
SHA-256:

```text
24042bd37915805615e6cf969ac57c6439124c3fe85823327f5f3fb24bd9ffea
```

That version targets .NET 9, so the automated workflow must install .NET 9
explicitly rather than rely on the runner image.

### 4. Select the stable release

Use the newest published, non-draft, non-prerelease GitComet release. At the
time this plan was written, that is `0.2.1`:

```text
https://github.com/Auto-Explore/GitComet/releases/download/v0.2.1/gitcomet-v0.2.1-windows-x86_64.msi
https://github.com/Auto-Explore/GitComet/releases/download/v0.2.1/gitcomet-v0.2.1-windows-arm64.msi
```

If a newer stable release exists when this work is performed, replace `0.2.1`
in the version and both URLs with that release.

### 5. Generate the initial manifest

Run WingetCreate interactively with both MSI URLs:

```powershell
wingetcreate new `
  "https://github.com/Auto-Explore/GitComet/releases/download/v0.2.1/gitcomet-v0.2.1-windows-x86_64.msi" `
  "https://github.com/Auto-Explore/GitComet/releases/download/v0.2.1/gitcomet-v0.2.1-windows-arm64.msi"
```

Create a multi-file manifest set under this upstream path:

```text
manifests/a/AutoExplore/GitComet/0.2.1/
```

The set must contain exactly:

- `AutoExplore.GitComet.yaml`
- `AutoExplore.GitComet.installer.yaml`
- `AutoExplore.GitComet.locale.en-US.yaml`

Use manifest schema `1.12.0` and the following package metadata:

| Field | Value |
|---|---|
| `PackageIdentifier` | `AutoExplore.GitComet` |
| `Publisher` | `AutoExplore Oy` |
| `Author` | `AutoExplore Oy` |
| `PackageName` | `GitComet` |
| `Moniker` | `gitcomet` |
| `PackageLocale` | `en-US` |
| `PublisherUrl` | `https://autoexplore.ai` |
| `PackageUrl` | `https://gitcomet.dev` |
| `PublisherSupportUrl` | `https://github.com/Auto-Explore/GitComet/issues` |
| `License` | `AGPL-3.0-only` |
| `LicenseUrl` | Version-pinned GitHub URL for `LICENSE-AGPL-3.0` |
| `ShortDescription` | `Fast, resource-efficient Git GUI written in Rust` |
| Tags | `git`, `git-client`, `git-gui`, `version-control`, `developer-tools` |

Review the installer manifest and ensure it contains:

- `InstallerType: wix`
- `Scope: machine`
- `UpgradeBehavior: install`
- `Commands: [gitcomet]`, expressed as normal YAML list syntax
- One `Architecture: x64` entry for the `windows-x86_64.msi` URL
- One `Architecture: arm64` entry for the `windows-arm64.msi` URL
- The SHA-256 and `ProductCode` extracted separately from each MSI
- Apps & Features metadata matching `GitComet`, `AutoExplore Oy`, and the
  package version
- Upgrade code `{3A9166E5-EB30-45B6-8128-C19FEA3A26DF}` in the applicable
  Apps & Features entries
- The default install location `%ProgramFiles%\GitComet`, if WingetCreate
  extracts installation metadata
- This dependency:

  ```yaml
  Dependencies:
    PackageDependencies:
      - PackageIdentifier: Git.Git
        MinimumVersion: 2.50.0
  ```

Do not add custom silent switches for the WiX MSI unless validation proves they
are necessary. WinGet knows the standard WiX/MSI switches.

### 6. Validate and test the initial manifest

Validate the complete manifest directory:

```powershell
winget validate --manifest <manifest-directory>
```

Enable local manifest testing from an elevated terminal:

```powershell
winget settings --enable LocalManifestFiles
winget install --manifest <manifest-directory>
```

Test on matching native x64 and ARM64 Windows systems. For each architecture:

1. Start without Git installed, or with Git older than 2.50, and confirm the
   `Git.Git` dependency is installed or upgraded.
2. Confirm installation completes without application-level interaction. The
   expected machine-scope UAC prompt is acceptable.
3. Open a new terminal and run:

   ```powershell
   gitcomet --version
   ```

4. Confirm Windows Apps & Features reports `GitComet`, publisher
   `AutoExplore Oy`, and the expected version.
5. Install the previous GitComet MSI and then install through the new local
   manifest to exercise major-upgrade behavior.
6. Uninstall and confirm the application, Start menu entry, and PATH entry are
   removed.

Windows Sandbox and the upstream `Tools/SandboxTest.ps1` helper are suitable for
the x64 clean-install test. Use a native Windows ARM64 machine or runner for the
ARM64 MSI.

### 7. Submit and monitor the initial PR

Submit only the single GitComet version and its three manifest files. Use a
standard title:

```text
New package: AutoExplore.GitComet version 0.2.1
```

Monitor all validation labels, complete the CLA, and respond to moderator
feedback. Do not add GitComet documentation or unrelated upstream changes to
the manifest PR.

After merge and catalog publication, verify:

```powershell
winget source update
winget show --exact --id AutoExplore.GitComet
winget install --exact --id AutoExplore.GitComet
winget uninstall --exact --id AutoExplore.GitComet
```

Do not advertise the command or activate release automation until `winget
show` succeeds.

## Phase 2: Automate stable release submissions

### New reusable workflow

Add `.github/workflows/deploy-winget.yml` with both `workflow_call` and
`workflow_dispatch` entry points.

Public workflow inputs:

- `tag`: release tag such as `v0.2.2`
- `version`: package version such as `0.2.2`
- `dry_run`: generate and upload manifests without opening a PR

Secret:

- `WINGET_CREATE_GITHUB_TOKEN`: optional for dry runs, required for live
  submissions

Use `windows-latest`, `permissions: contents: read`, a 20-minute timeout, and a
per-version concurrency group with `cancel-in-progress: false`.

### Input and release preflight

The workflow must:

1. Normalize an optional leading `v` and require the exact relationship
   `tag == v<version>`.
2. Accept only stable `X.Y.Z` versions. Reject `-rc.N` and all other suffixes.
3. Query the GitHub release and reject drafts or prereleases.
4. Require exactly these two assets:
   - `gitcomet-v<version>-windows-x86_64.msi`
   - `gitcomet-v<version>-windows-arm64.msi`
5. Require a non-empty `WINGET_CREATE_GITHUB_TOKEN` only when
   `dry_run == false`.

Use the `browser_download_url` values returned by the release API. Do not make
WinGet publication depend on the Azure-hosted Windows installer or APT
deployment.

### Pinned tooling

Install .NET 9 with the same pinned `actions/setup-dotnet` action already used
by the Microsoft Store workflow. Download this exact tool:

```text
https://github.com/microsoft/winget-create/releases/download/v1.12.13.0/wingetcreate.exe
```

Fail if its SHA-256 does not match the value recorded above. Run
`wingetcreate info` after verification so the tool/runtime versions appear in
the log.

### Manifest generation

Generate the next manifest into a temporary output directory with
`wingetcreate update AutoExplore.GitComet`. Provide:

- `--version <version>`
- Both installer URLs
- Architecture and scope overrides:
  - `<x64-url>|x64|machine`
  - `<arm64-url>|arm64|machine`
- `--release-date <YYYY-MM-DD>` from the GitHub release
- `--release-notes-url https://github.com/Auto-Explore/GitComet/releases/tag/<tag>`
- `--out <temporary-directory>`

Upload the generated files as `winget-manifests-<version>` in both dry-run and
live modes. Verify the generated set still contains the correct package ID,
version, two architectures, exact URLs, machine scope, WiX installer type, and
Git 2.50 dependency before submission.

### Idempotence and submission

Before generation or submission, query the upstream repository and open PRs:

- If this exact version is already merged, finish successfully and report its
  manifest URL.
- If an open PR already targets this exact version, finish successfully and
  report the PR URL.
- If another version of `AutoExplore.GitComet` has an open PR, fail with that
  PR's URL. Do not automatically close or overwrite an external PR.

For a live run, expose the secret only on the submission step:

```yaml
env:
  WINGET_CREATE_GITHUB_TOKEN: ${{ secrets.WINGET_CREATE_GITHUB_TOKEN }}
```

Submit the generated manifest directory with `wingetcreate submit`,
`--no-open`, and this PR title:

```text
New version: AutoExplore.GitComet version <version>
```

Query the resulting open PR and put its URL in the GitHub Actions job summary.
A successful workflow means that the PR was opened, not that Microsoft has
merged or published it.

### Main release pipeline integration

Add a `deploy_winget` job to
`.github/workflows/release-manual-main.yml`. It should:

- Need `validate`, `build_and_upload`, and `publish_release`
- Run only when the release is not a draft
- Run only when the release is not marked as a prerelease
- Skip any version containing an RC suffix as a defense in depth
- Require the release build and publication to have succeeded
- Call `./.github/workflows/deploy-winget.yml`
- Pass `tag`, `version`, and `dry_run: false`
- Use `secrets: inherit`

Keep the job parallel with Homebrew, AUR, and APT deployment. It must not be a
dependency of Azure Windows installer or Microsoft Store deployment. A WinGet
failure should leave the release published, make the overall workflow visibly
red, and allow unrelated deployment jobs to continue.

## Phase 3: CI and documentation

Extend `.github/workflows/deployment-ci.yml` so changes to the WinGet workflow
trigger Deployment CI. Add the workflow to explicit YAML validation and assert
the presence of:

- `AutoExplore.GitComet`
- `WINGET_CREATE_GITHUB_TOKEN`
- Stable-only and non-draft guards
- Both exact MSI name patterns
- `|x64|machine` and `|arm64|machine`
- WingetCreate version and SHA-256 pin
- `--release-date`, `--release-notes-url`, `--no-open`, and the standard PR
  title
- Dry-run behavior that never invokes submission

Add `docs/winget-release.md` as the permanent maintainer guide derived from
this plan. It should cover initial onboarding, credentials, normal releases,
manual dry runs and backfills, token rotation, upstream moderation, and these
recovery cases:

- PAT expired or missing
- CLA not accepted by the PAT owner
- WingetCreate fork synchronization or HTTP 422 failure
- Existing or stale upstream PR
- Missing release asset
- WingetCreate checksum mismatch
- Manifest validation or MSI metadata mismatch

Once the initial package is searchable, add this to the Windows download
section in `README.md`:

```powershell
winget install --exact --id AutoExplore.GitComet
```

## Acceptance checklist

- [ ] Initial upstream package has a three-file schema 1.12.0 manifest.
- [ ] Manifest has exactly x64 and ARM64 machine-scoped WiX installers.
- [ ] Both installer hashes, product codes, publisher, version, and upgrade
      correlation match the signed MSIs.
- [ ] `Git.Git >= 2.50.0` is installed as a package dependency.
- [ ] Clean install, previous-version upgrade, CLI version check, and uninstall
      pass on x64 Windows.
- [ ] The same scenarios pass on native ARM64 Windows.
- [ ] The initial upstream PR passes validation, CLA, moderation, and publishing.
- [ ] `winget show --exact --id AutoExplore.GitComet` succeeds publicly.
- [ ] Dry-run automation generates and uploads correct manifests without a PAT.
- [ ] A stable release opens exactly one upstream update PR.
- [ ] Rerunning the same release does not create a duplicate PR.
- [ ] A prerelease never opens an `AutoExplore.GitComet` PR.
- [ ] WinGet failure does not block Azure installer or Microsoft Store jobs.
- [ ] README advertises WinGet only after the package is publicly discoverable.

## Defaults and boundaries

- The existing WiX MSI remains the WinGet payload.
- Portable ZIP and Microsoft Store packages are out of scope for WinGet.
- Prereleases are not published; there is no preview package ID.
- The initial upstream manifest must be accepted before automation is activated.
- External moderation delay does not block or roll back a GitHub release.
- Updating WingetCreate requires updating and reviewing its checksum in the
  same change.
