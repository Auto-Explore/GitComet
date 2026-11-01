<#
.SYNOPSIS
Build GitComet with startup allocation counters and debug symbols.
.DESCRIPTION
Uses the release-with-debug profile and perf-alloc feature. The separate
target/alloc-profile directory preserves the ordinary CPU-profiling build.
Allocation instrumentation adds overhead; use this build for allocation counts.
.PARAMETER TargetDirectory
Cargo output directory. Relative paths are resolved from the repository root.
.EXAMPLE
.\scripts\build-gitcomet-allocations.ps1
#>
[CmdletBinding()]
param(
    [string]$TargetDirectory = 'target/alloc-profile'
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
if (-not [System.IO.Path]::IsPathRooted($TargetDirectory)) {
    $TargetDirectory = Join-Path $repoRoot $TargetDirectory
}
$targetPath = [System.IO.Path]::GetFullPath($TargetDirectory)

Push-Location -LiteralPath $repoRoot
try {
    & cargo build -p gitcomet --bin gitcomet --profile release-with-debug `
        --features perf-alloc --target-dir $targetPath
    if ($LASTEXITCODE -ne 0) {
        throw "GitComet allocation build failed with exit code $LASTEXITCODE."
    }
} finally {
    Pop-Location
}

Write-Output ("Allocation build: " + (Join-Path $targetPath 'release-with-debug/gitcomet.exe'))
