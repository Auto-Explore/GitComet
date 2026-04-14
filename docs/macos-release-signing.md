# macOS Release Signing

This repository already contains the basic macOS release flow:

- [`scripts/package-macos.sh`](../scripts/package-macos.sh) builds the macOS app bundle, signs the bundled executable and app when a signing identity is provided, and emits the macOS `.tar.gz` and `.dmg` artifacts.
- [`scripts/notarize-macos.sh`](../scripts/notarize-macos.sh) submits the packaged DMG to Apple's notary service, staples the resulting ticket to the staged `.app` bundle and DMG, validates the stapled artifacts, and rebuilds the tarball so the archived `.app` bundle is current.
- [`.github/workflows/build-release-artifacts.yml`](../.github/workflows/build-release-artifacts.yml) wires the same process into release CI.

## What You Need

For outside-the-App-Store distribution you need:

1. An Apple Developer Program team.
2. A `Developer ID Application` certificate with the private key available to `codesign`.
3. A notarization credential for `notarytool`.
   Recommended in this repo: an App Store Connect API key.
   Local alternative: a stored `notarytool` keychain profile.

Check whether the current shell can see a usable signing identity:

```bash
security find-identity -v -p codesigning
```

For release signing, this must show a valid `Developer ID Application: ...` identity. If it shows `0 valid identities found`, fix the certificate/private-key installation in Keychain Access before trying to sign.

## Local Release Flow

First, verify that macOS can see the correct signing identity:

```bash
security find-identity -v -p codesigning
```

You should see a valid `Developer ID Application: ...` entry.

Build and sign on each native architecture you ship:

```bash
scripts/package-macos.sh \
  --version 0.2.0 \
  --arch arm64 \
  --release \
  --codesign-identity "Developer ID Application: Example, Inc. (TEAMID)"
```

Then notarize and staple the generated artifacts:

```bash
scripts/notarize-macos.sh \
  --version 0.2.0 \
  --arch arm64 \
  --keychain-profile gitcomet-notary
```

If you prefer direct API-key authentication instead of a stored profile:

```bash
scripts/notarize-macos.sh \
  --version 0.2.0 \
  --arch arm64 \
  --api-key /path/to/AuthKey_ABC1234567.p8 \
  --key-id ABC1234567 \
  --issuer 00000000-0000-0000-0000-000000000000
```

To create a local keychain profile up front:

```bash
xcrun notarytool store-credentials gitcomet-notary \
  --key /path/to/AuthKey_ABC1234567.p8 \
  --key-id ABC1234567 \
  --issuer 00000000-0000-0000-0000-000000000000
```

Repeat the same packaging and notarization flow on Intel macOS for `--arch x86_64`.

## Local Verification Checklist

The packaging and notarization scripts already run the important checks for you:

- [`scripts/package-macos.sh`](../scripts/package-macos.sh) signs with Hardened Runtime and runs `codesign --verify` on the app and standalone binary.
- [`scripts/notarize-macos.sh`](../scripts/notarize-macos.sh) submits the DMG with `notarytool`, staples the `.app` and `.dmg`, validates stapling, then runs `codesign --verify` and `spctl --assess`.

After a successful local run, you can manually spot-check the finished artifacts:

```bash
codesign --verify --deep --strict --verbose=2 dist/stage/gitcomet-v0.2.0-macos-arm64/GitComet.app
spctl --assess --type open --context context:primary-signature --verbose=4 dist/stage/gitcomet-v0.2.0-macos-arm64/GitComet.app
spctl --assess --type open --context context:primary-signature --verbose=4 dist/gitcomet-v0.2.0-macos-arm64.dmg
xcrun stapler validate dist/stage/gitcomet-v0.2.0-macos-arm64/GitComet.app
xcrun stapler validate dist/gitcomet-v0.2.0-macos-arm64.dmg
```

If all of those commands return success, the signed macOS app bundle and DMG are in good shape.

## GitHub Setup

Use repository-level or organization-level GitHub Actions secrets for the macOS signing values used by the reusable release workflow.

1. Export the `Developer ID Application` identity from Keychain Access as a `.p12`.
2. Choose an export password for that `.p12`.
3. Base64-encode the `.p12` as a single line:

```bash
base64 -i gitcomet-signing.p12 | tr -d '\n'
```

4. In GitHub, add these Actions secrets:

- `MACOS_SIGNING_IDENTITY`
  The exact identity string, for example `Developer ID Application: Example, Inc. (TEAMID)`.
- `MACOS_SIGNING_CERT_BASE64`
  The single-line base64 output of the exported `.p12`.
- `MACOS_SIGNING_CERT_PASSWORD`
  The password you set when exporting the `.p12`.
- `MACOS_NOTARY_KEY_ID`
  The App Store Connect API key ID.
- `MACOS_NOTARY_ISSUER_ID`
  The App Store Connect issuer UUID.
- `MACOS_NOTARY_API_KEY_P8`
  The full contents of the downloaded `AuthKey_<KEYID>.p8` file.

5. Trigger the release workflow from [`release-manual-main.yml`](../.github/workflows/release-manual-main.yml) on `main`.

## App Store Connect Notary Key

This repository's CI uses an App Store Connect API key for notarization.

Create a Team API key, download the `.p8` file once, and store the key material securely. The key ID and issuer ID become GitHub secrets, and the `.p8` contents become `MACOS_NOTARY_API_KEY_P8`.

For local testing you can either:

- use the same `.p8` file directly with [`scripts/notarize-macos.sh`](../scripts/notarize-macos.sh), or
- save the credentials into your macOS keychain with `xcrun notarytool store-credentials`.

## CI Secrets

Release CI enables macOS signing only when all of these secrets are present:

- `MACOS_SIGNING_IDENTITY`
- `MACOS_SIGNING_CERT_BASE64`
- `MACOS_SIGNING_CERT_PASSWORD`
- `MACOS_NOTARY_KEY_ID`
- `MACOS_NOTARY_ISSUER_ID`
- `MACOS_NOTARY_API_KEY_P8`

The workflow imports the `.p12` certificate into a temporary keychain, signs the macOS artifacts, notarizes the DMG, staples the `.app` and `.dmg`, and uploads the resulting release assets.

## Artifact Expectations

Treat the DMG as the canonical trusted macOS download.

- The DMG is the notarized and stapled end-user artifact.
- The tarball is rebuilt after stapling so the bundled `GitComet.app` is current.
- The standalone `gitcomet` binary at the tarball root is only code-signed. Apple's notary service does not accept `.tar.gz` uploads directly, and that file is not inside the submitted DMG.

If you want a separately trusted CLI-only macOS artifact, publish it in a notary-supported container such as a ZIP, DMG, or signed flat PKG and update downstream packaging accordingly.
