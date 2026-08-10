<#
.SYNOPSIS
  Builds the GitComet desktop app for Windows and optionally packages a
  portable ZIP, mirroring the CI release build
  (.github/workflows/build-release-artifacts.yml).

.DESCRIPTION
  Checks the Windows prerequisites (Rust toolchain, MSVC linker, Windows SDK,
  disk space), then runs:

    cargo build -p gitcomet --release --locked --features ui-gpui,gix --bin gitcomet

  The repository's .cargo/config.toml already points the linker at
  scripts/windows/msvc-linker.cmd, so no shell setup is required beyond having
  Visual Studio Build Tools / Community installed.

  The script prepends C:\Windows\System32 to PATH so Windows tools always
  win over GNU coreutils (e.g. from Git Bash) that otherwise shadow sort.exe
  and break the MSVC linker script.

.PARAMETER Configuration
  Build profile: Release (default), Debug, or ReleaseWithDebug.

.PARAMETER Arch
  Architecture to build for: host (default), x64, or arm64. Cross-compiling
  requires the matching rustup target, e.g. `rustup target add
  aarch64-pc-windows-msvc`.

.PARAMETER Features
  Cargo feature list for the gitcomet crate. Defaults to "ui-gpui,gix" (the
  full GUI app used by the release pipeline). Pass an empty string to build
  with the crate's default features.

.PARAMETER Package
  Also create the portable ZIP under dist/ (gitcomet-v<version>-windows-<arch>
  -portable.zip) with the binary, README, LICENSE, and NOTICE.

.PARAMETER Msi
  Also build the WiX MSI installer under dist/
  (gitcomet-v<version>-windows-<arch>.msi). Installs cargo-wix and the WiX
  Toolset automatically when missing (WiX installs machine-wide and may
  prompt for elevation).

.PARAMETER SkipLocked
  Do not pass --locked to cargo (use this if Cargo.lock is out of date).

.PARAMETER MinFreeSpaceGb
  Minimum free disk space (GB) required on the drive hosting the repo before
  the build starts. Defaults to 10.

.PARAMETER InstallVersion
  Overrides the version embedded in the MSI (cargo-wix --install-version)
  without touching Cargo.toml. Useful for pre-release/upgrade testing.

.EXAMPLE
  .\scripts\windows\build.ps1

  Build the release binary at target\release\gitcomet.exe.

.EXAMPLE
  .\scripts\windows\build.ps1 -Package -Arch arm64

  Cross-compile for ARM64 and emit the portable ZIP.

.EXAMPLE
  .\scripts\windows\build.ps1 -Package -Msi

  Build the release binary plus the portable ZIP and the MSI installer,
  matching the CI release artifacts.
#>
[CmdletBinding()]
param(
  [ValidateSet("Release", "Debug", "ReleaseWithDebug")]
  [string]$Configuration = "Release",

  [ValidateSet("host", "x64", "arm64")]
  [string]$Arch = "host",

  [AllowEmptyString()]
  [string]$Features = "ui-gpui,gix",

  [switch]$Package,

  [switch]$Msi,

  [switch]$SkipLocked,

  [int]$MinFreeSpaceGb = 10,

  [string]$InstallVersion = ""
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

# ── Helpers ────────────────────────────────────────────────────────────────
function Write-Step {
  param([Parameter(Mandatory = $true)][string]$Message)
  Write-Host ""
  Write-Host "==> $Message" -ForegroundColor Cyan
}

function Assert-CommandOnPath {
  param(
    [Parameter(Mandatory = $true)][string]$Name,
    [Parameter(Mandatory = $true)][string]$Hint
  )
  if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
    throw "'$Name' was not found on PATH.`n$Hint"
  }
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$targetDir = Join-Path $repoRoot "target"
$stopwatch = [System.Diagnostics.Stopwatch]::StartNew()

# Windows tools first: GNU coreutils shipped with Git for Windows shadow
# sort.exe (msvc-linker.cmd) and other tools when their bin dirs precede
# System32 on PATH.
$env:PATH = "$([Environment]::SystemDirectory);$env:PATH"

# ── Architecture ───────────────────────────────────────────────────────────
$cargoTarget = $null
$archLabel = $Arch
$linkerArch = $Arch

if ($Arch -eq "host") {
  $isArm64 = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -eq [System.Runtime.InteropServices.Architecture]::Arm64
  $archLabel = if ($isArm64) { "arm64" } else { "x64" }
}

if ($archLabel -eq "arm64") {
  $cargoTarget = "aarch64-pc-windows-msvc"
  $linkerArch = "arm64"
} elseif ($archLabel -eq "x64") {
  $cargoTarget = "x86_64-pc-windows-msvc"
  $linkerArch = "x64"
}

# ── Prerequisite checks ────────────────────────────────────────────────────
Write-Step "Checking prerequisites (architecture: $archLabel, configuration: $Configuration)..."

Assert-CommandOnPath "cargo" "Install the Rust toolchain with https://rustup.rs and reopen your shell."
Assert-CommandOnPath "rustc" "Install the Rust toolchain with https://rustup.rs and reopen your shell."

$cargoVersion = (& cargo --version | Select-Object -First 1).Trim()
Write-Host "  - cargo:    $cargoVersion"

if ($cargoTarget) {
  $installedTargets = & rustup target list --installed 2>$null
  if ($LASTEXITCODE -ne 0 -or ($installedTargets -notcontains $cargoTarget)) {
    throw "Rust target '$cargoTarget' is not installed. Run: rustup target add $cargoTarget"
  }
}

$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path -LiteralPath $vswhere)) {
  throw "Visual Studio Installer was not found at '$vswhere'.`nInstall Visual Studio 2022 (Community or Build Tools) with the 'Desktop development with C++' workload (MSVC tools + Windows 10/11 SDK)."
}

$vsComponent = if ($linkerArch -eq "arm64") {
  "Microsoft.VisualStudio.Component.VC.Tools.ARM64"
} else {
  "Microsoft.VisualStudio.Component.VC.Tools.x86.x64"
}
$vsInstall = & $vswhere -latest -products * -requires $vsComponent -property installationPath 2>$null
if ([string]::IsNullOrWhiteSpace($vsInstall)) {
  throw "No Visual Studio installation with the MSVC $linkerArch tools was found.`nInstall the 'Desktop development with C++' workload (component: $vsComponent) in Visual Studio Installer."
}

# MSVC toolset must contain at least one version directory.
$msvcTools = Join-Path $vsInstall "VC\Tools\MSVC"
$msvcVersions = @(Get-ChildItem -LiteralPath $msvcTools -Directory -ErrorAction SilentlyContinue | Sort-Object Name -Descending)
if ($msvcVersions.Count -eq 0) {
  throw "No MSVC toolset found under '$msvcTools'.`nRe-run the Visual Studio Installer and add the C++ (MSVC) tools, then repair the installation."
}
Write-Host "  - MSVC:     $vsInstall (toolset $($msvcVersions[0].Name))"

# Windows SDK must ship kernel32.lib for the target architecture.
$sdkRoot = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\Lib"
if (-not (Test-Path -LiteralPath $sdkRoot)) {
  throw "Windows SDK not found at '$sdkRoot'.`nInstall the Windows 10/11 SDK component in Visual Studio Installer."
}
$sdkFound = $false
foreach ($sdkVer in (Get-ChildItem -LiteralPath $sdkRoot -Directory -ErrorAction SilentlyContinue | Sort-Object Name -Descending)) {
  if (Test-Path -LiteralPath (Join-Path $sdkVer.FullName "um\$linkerArch\kernel32.lib")) {
    Write-Host "  - Windows SDK: $sdkRoot ($($sdkVer.Name))"
    $sdkFound = $true
    break
  }
}
if (-not $sdkFound) {
  throw "Windows SDK $linkerArch libraries missing (kernel32.lib not found under '$sdkRoot').`nInstall the Windows 10/11 SDK component in Visual Studio Installer."
}

# Free disk space on the drive hosting the repo (a release build needs several GB).
$repoDrive = (Get-Item -LiteralPath $repoRoot).PSDrive
$freeGb = [math]::Round($repoDrive.Free / 1GB, 1)
Write-Host "  - disk:     $freeGb GB free on $($repoDrive.Name):\"
if ($freeGb -lt $MinFreeSpaceGb) {
  throw "Only $freeGb GB free on $($repoDrive.Name):\ but the build requires at least $MinFreeSpaceGb GB."
}

# ── Build ──────────────────────────────────────────────────────────────────
Write-Step "Building gitcomet ($(if ($Features) { $Features } else { 'default features' }))..."
Write-Host "  This is a large GpUI application; a first Release build can take 20-40 minutes."

# scripts/windows/msvc-linker.cmd reads this to pick the MSVC/SDK architecture.
$env:GITCOMET_TARGET_ARCH = $linkerArch

$cargoArgs = @("build", "-p", "gitcomet", "--bin", "gitcomet")
if ($Features) {
  $cargoArgs += "--features", $Features
}
switch ($Configuration) {
  "Release"          { $cargoArgs += "--release" }
  "ReleaseWithDebug" { $cargoArgs += "--profile", "release-with-debug" }
  "Debug"            { } # default dev profile
}
if (-not $SkipLocked) {
  $cargoArgs += "--locked"
}
if ($cargoTarget) {
  $cargoArgs += "--target", $cargoTarget
}

Write-Host ""
Write-Host "> cargo $($cargoArgs -join ' ')" -ForegroundColor DarkGray
& cargo @cargoArgs
if ($LASTEXITCODE -ne 0) {
  Write-Host ""
  Write-Host "cargo build failed with exit code $LASTEXITCODE." -ForegroundColor Red
  Write-Host "Common causes on Windows:" -ForegroundColor Yellow
  Write-Host "  - MSVC/SDK components missing: run 'Visual Studio Installer' > Modify and check 'Desktop development with C++'."
  Write-Host "  - MSVC linker misconfigured: run scripts\windows\msvc-linker.cmd manually and inspect its error."
  Write-Host "  - GNU tools (Git Bash) shadowing Windows tools: launch from a plain PowerShell/CMD, or ensure System32 precedes Git's bin dirs on PATH."
  Write-Host "  - Out of disk space: the build needs several GB under target\."
  throw "cargo build failed with exit code $LASTEXITCODE."
}

$profileDir = switch ($Configuration) {
  "Release"          { "release" }
  "ReleaseWithDebug" { "release-with-debug" }
  "Debug"            { "debug" }
}
if ($cargoTarget) {
  $binaryPath = Join-Path $targetDir (Join-Path $cargoTarget (Join-Path $profileDir "gitcomet.exe"))
} else {
  $binaryPath = Join-Path $targetDir (Join-Path $profileDir "gitcomet.exe")
}

if (-not (Test-Path -LiteralPath $binaryPath)) {
  throw "Expected binary not found at '$binaryPath'."
}

# ── Verify output ──────────────────────────────────────────────────────────
$sizeMb = [math]::Round((Get-Item -LiteralPath $binaryPath).Length / 1MB, 1)
$binaryHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $binaryPath).Hash
$binaryVersion = (& $binaryPath --version 2>$null | Select-Object -First 1)
if ([string]::IsNullOrWhiteSpace($binaryVersion)) {
  $binaryVersion = "(version check failed)"
}

Write-Host ""
Write-Host "Build succeeded:" -ForegroundColor Green
Write-Host "  - binary:  $binaryPath ($sizeMb MiB)"
Write-Host "  - version: $binaryVersion"
Write-Host "  - sha256:  $binaryHash"

# ── Optional portable packaging ────────────────────────────────────────────
if ($Package -or $Msi) {
  $cargoToml = Get-Content -Raw (Join-Path $repoRoot "Cargo.toml")
  $versionMatch = [regex]::Match($cargoToml, '(?ms)\[workspace\.package\][^\[]*?version\s*=\s*"([^"]+)"')
  if (-not $versionMatch.Success) {
    throw "Could not determine the workspace version from Cargo.toml."
  }
  $version = $versionMatch.Groups[1].Value

  $distDir = Join-Path $repoRoot "dist"
}

if ($Package) {
  Write-Step "Packaging portable ZIP..."

  $portableDir = Join-Path $distDir "portable"
  $zipName = "gitcomet-v${version}-windows-${archLabel}-portable.zip"
  $zipPath = Join-Path $distDir $zipName

  New-Item -ItemType Directory -Path $portableDir -Force | Out-Null
  Copy-Item -LiteralPath $binaryPath -Destination (Join-Path $portableDir "gitcomet.exe") -Force
  Copy-Item -LiteralPath (Join-Path $repoRoot "README.md") -Destination (Join-Path $portableDir "README.md") -Force
  Copy-Item -LiteralPath (Join-Path $repoRoot "LICENSE-AGPL-3.0") -Destination (Join-Path $portableDir "LICENSE-AGPL-3.0") -Force
  Copy-Item -LiteralPath (Join-Path $repoRoot "NOTICE") -Destination (Join-Path $portableDir "NOTICE") -Force

  if (Test-Path -LiteralPath $zipPath) {
    Remove-Item -LiteralPath $zipPath -Force
  }
  Compress-Archive -Path (Join-Path $portableDir "*") -DestinationPath $zipPath

  $zipMb = [math]::Round((Get-Item -LiteralPath $zipPath).Length / 1MB, 1)
  $zipHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $zipPath).Hash
  Write-Host ""
  Write-Host "Packaged:  $zipPath ($zipMb MiB)" -ForegroundColor Green
  Write-Host "  - sha256: $zipHash"
}

# ── Optional MSI installer (WiX) ───────────────────────────────────────────
if ($Msi) {
  Write-Step "Building MSI installer (WiX)..."

  # cargo-wix (matches the CI version from .github/workflows/build-release-artifacts.yml).
  $cargoWixVersion = "0.3.9"
  if (-not (Get-Command cargo-wix -ErrorAction SilentlyContinue)) {
    Write-Host "cargo-wix not found; installing cargo-wix $cargoWixVersion (compiles from source, may take a few minutes)..."
    & cargo install cargo-wix --version $cargoWixVersion --locked
    if ($LASTEXITCODE -ne 0) {
      throw "cargo install cargo-wix failed with exit code $LASTEXITCODE."
    }
    if (-not (Get-Command cargo-wix -ErrorAction SilentlyContinue)) {
      throw "cargo-wix was installed but is not on PATH. Ensure ~\.cargo\bin is on PATH and re-run."
    }
  }

  # Locate the WiX Toolset (candle.exe).
  $wixBin = $null
  $candle = Get-Command candle.exe -ErrorAction SilentlyContinue
  if ($candle) {
    $wixBin = Split-Path $candle.Source -Parent
  }
  if (-not $wixBin) {
    $wixMachine = [Environment]::GetEnvironmentVariable("WIX", "Machine")
    if ($wixMachine) {
      foreach ($c in @((Join-Path $wixMachine "bin\candle.exe"), (Join-Path $wixMachine "candle.exe"))) {
        if (Test-Path -LiteralPath $c) { $wixBin = Split-Path $c -Parent; break }
      }
    }
  }
  if (-not $wixBin) {
    foreach ($root in @(${env:ProgramFiles(x86)}, $env:ProgramFiles)) {
      if (-not $root) { continue }
      $c = Join-Path $root "WiX Toolset v3.14\bin\candle.exe"
      if (Test-Path -LiteralPath $c) { $wixBin = Split-Path $c -Parent; break }
    }
  }

  # Not found: try to install it (winget prefers, choco as fallback).
  if (-not $wixBin) {
    Write-Host "WiX Toolset not found. Attempting to install it..." -ForegroundColor Yellow
    if (Get-Command winget -ErrorAction SilentlyContinue) {
      # winget may prompt for elevation (UAC) since WiX installs machine-wide.
      & winget install --id WiXToolset.WiXToolset --exact --silent --accept-package-agreements --accept-source-agreements
      if ($LASTEXITCODE -ne 0) {
        throw "winget failed to install WiX Toolset (exit code $LASTEXITCODE). Install 'WiX Toolset 3.14' manually from https://wixtoolset.org/releases/ or run this script elevated."
      }
    } elseif (Get-Command choco -ErrorAction SilentlyContinue) {
      & choco install wixtoolset --version 3.14.1.20250415 --yes --no-progress
      if ($LASTEXITCODE -ne 0) {
        throw "choco failed to install WiX Toolset (exit code $LASTEXITCODE)."
      }
    } else {
      throw "WiX Toolset 3.14 is required for MSI builds but was not found, and neither winget nor choco is available. Install it manually from https://wixtoolset.org/releases/."
    }
    $wixBin = $null
    $candle = Get-Command candle.exe -ErrorAction SilentlyContinue
    if ($candle) { $wixBin = Split-Path $candle.Source -Parent }
    if (-not $wixBin) {
      throw "WiX Toolset was installed but candle.exe could not be located. Reopen your shell so the updated PATH takes effect, then re-run."
    }
  }

  $env:WIX = Split-Path $wixBin -Parent
  $env:PATH = "$wixBin;$env:PATH"
  Write-Host "  - WiX:       $wixBin"

  $wxsPath = Join-Path $repoRoot "crates\gitcomet\wix\main.wxs"
  if (-not (Test-Path -LiteralPath $wxsPath)) {
    Write-Host "wix\main.wxs missing; running 'cargo wix init'..."
    & cargo-wix init --package gitcomet
    if ($LASTEXITCODE -ne 0) {
      throw "cargo wix init failed with exit code $LASTEXITCODE."
    }
  }

  $msiName = "gitcomet-v${version}-windows-${archLabel}.msi"
  $msiPath = Join-Path $distDir $msiName
  if ($InstallVersion) {
    # Override the version embedded in the MSI without touching Cargo.toml.
    $msiName = "gitcomet-v${InstallVersion}-windows-${archLabel}.msi"
    $msiPath = Join-Path $distDir $msiName
  }
  if (Test-Path -LiteralPath $msiPath) {
    Remove-Item -LiteralPath $msiPath -Force
  }

  # The binary lives under target\<triple>\<profile> because the build above
  # passes --target; tell cargo-wix exactly where so --no-build finds it.
  $wixProfile = switch ($Configuration) {
    "Release"          { "release" }
    "ReleaseWithDebug" { "release-with-debug" }
    "Debug"            { "debug" }
  }
  $wixArgs = @("--package", "gitcomet", "--profile", $wixProfile, "--nocapture", "--no-build")
  if ($InstallVersion) {
    $wixArgs += "--install-version", $InstallVersion
  }
  if ($cargoTarget) {
    $wixArgs += "--target", $cargoTarget
    $wixArgs += "--target-bin-dir", (Split-Path $binaryPath -Parent)
  }
  $wixArgs += "--bin-path", $wixBin
  $wixArgs += "--output", $msiPath

  Write-Host ""
  Write-Host "> cargo wix $($wixArgs -join ' ')" -ForegroundColor DarkGray
  # cargo-wix must be invoked as the 'cargo wix' subcommand: its binary only
  # accepts arguments when routed through cargo.
  & cargo wix @wixArgs
  if ($LASTEXITCODE -ne 0) {
    throw "cargo wix failed with exit code $LASTEXITCODE."
  }
  if (-not (Test-Path -LiteralPath $msiPath)) {
    throw "MSI was not produced at '$msiPath'."
  }

  $msiMb = [math]::Round((Get-Item -LiteralPath $msiPath).Length / 1MB, 1)
  $msiHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $msiPath).Hash
  Write-Host ""
  Write-Host "MSI:        $msiPath ($msiMb MiB)" -ForegroundColor Green
  Write-Host "  - sha256:  $msiHash"
}

# ── Summary ────────────────────────────────────────────────────────────────
$stopwatch.Stop()
$elapsed = $stopwatch.Elapsed
Write-Host ""
Write-Host "Done in $([math]::Round($elapsed.TotalMinutes, 1)) minutes." -ForegroundColor Green
