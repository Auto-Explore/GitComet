<#
.SYNOPSIS
Launch the release-with-debug GitComet build with startup-stage tracing.
.DESCRIPTION
For CPU profiling, first run record-gitcomet-startup.ps1 in another window and
wait for "Profiling all processes...". This script records startup stages using
the existing capture helper. It returns after the first interactive milestone,
which can precede repository readiness, and leaves GitComet running by default.
.EXAMPLE
.\scripts\capture-gitcomet-startup.ps1
.EXAMPLE
.\scripts\capture-gitcomet-startup.ps1 -EmptyProfile -AutoExit
#>
[CmdletBinding()]
param(
    [string]$Binary = '',
    [string]$OutputDirectory = '',
    [switch]$EmptyProfile,
    [switch]$AutoExit,
    [ValidateRange(1, 600)][int]$TimeoutSeconds = 90
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($Binary)) {
    $Binary = Join-Path $repoRoot 'target/release-with-debug/gitcomet.exe'
}
if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) {
    throw "Missing $Binary. Finish building GitComet with --profile release-with-debug."
}
if (-not $EmptyProfile -and (Get-Process -Name gitcomet -ErrorAction SilentlyContinue)) {
    throw 'Close existing GitComet windows before capturing a new startup.'
}
$captureArguments = @{
    Binary = $Binary
    EmptyProfile = $EmptyProfile
    AutoExit = $AutoExit
    TimeoutSeconds = $TimeoutSeconds
}
if (-not [string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $captureArguments.OutputDirectory = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDirectory)
}
& (Join-Path $PSScriptRoot 'profiling/capture-startup.ps1') @captureArguments
if (-not $AutoExit) {
    Write-Host 'Wait for repository status/history to finish loading before stopping samply in the recording window.'
}
