<#
.SYNOPSIS
Capture GitComet startup timings and allocation totals.
.DESCRIPTION
Build first with build-gitcomet-allocations.ps1 and close existing GitComet
windows. The default capture uses saved settings and leaves the app open.
Allow repository loading to finish, then close the app normally.

Allocation milestone totals are written to stderr.log with the prefix
GITCOMET_PERF_STARTUP_EVENT. Stage timings are in startup.jsonl. Allocation
totals do not include allocation call stacks or all native/GPU allocations.
.PARAMETER Binary
Allocation-enabled executable. Relative paths use the current directory.
.PARAMETER OutputDirectory
Capture directory. Omit to use a fresh directory under target/startup-captures.
.PARAMETER EmptyProfile
Use an isolated application profile to measure the empty Home picker.
.PARAMETER AutoExit
Request normal closure one second after the first interactive frame, or ready
Home picker with EmptyProfile. This does not wait for repository loading.
.PARAMETER TimeoutSeconds
Maximum time to observe the startup milestone; timeout leaves the app running.
.EXAMPLE
.\scripts\capture-gitcomet-allocations.ps1
.EXAMPLE
.\scripts\capture-gitcomet-allocations.ps1 -EmptyProfile -AutoExit
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
    $Binary = Join-Path $repoRoot 'target/alloc-profile/release-with-debug/gitcomet.exe'
}
if (-not (Test-Path -LiteralPath $Binary -PathType Leaf)) {
    throw "Missing $Binary. Run build-gitcomet-allocations.ps1 first."
}
if (-not $EmptyProfile -and (Get-Process -Name gitcomet -ErrorAction SilentlyContinue)) {
    throw 'Close existing GitComet windows before capturing a new startup.'
}
if (-not [string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDirectory)
}

$captureScript = Join-Path $PSScriptRoot 'profiling/capture-startup.ps1'
& $captureScript -Binary $Binary -OutputDirectory $OutputDirectory `
    -EmptyProfile:$EmptyProfile -AutoExit:$AutoExit -TimeoutSeconds $TimeoutSeconds
