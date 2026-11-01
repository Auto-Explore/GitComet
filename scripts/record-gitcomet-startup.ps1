<#
.SYNOPSIS
Record a detailed Windows samply profile of GitComet startup.
.DESCRIPTION
Finish building release-with-debug and close GitComet first. When samply prints
"Profiling all processes...", run capture-gitcomet-startup.ps1 in a second
PowerShell window. After repository status and history are usable, press Ctrl+C
once in the recording window. The app stays open.

The default 8000 Hz capture includes scheduling and graphics events and keeps
the raw ETL files. This is a diagnostic capture with significant overhead.
Samply 0.13.1 ignores --duration on Windows.
.EXAMPLE
.\scripts\record-gitcomet-startup.ps1
.EXAMPLE
.\scripts\record-gitcomet-startup.ps1 -Rate 1000
#>
[CmdletBinding()]
param(
    [ValidateRange(1, 8192)][int]$Rate = 8000,
    [string]$OutputDirectory = ''
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$binaryDirectory = Join-Path $repoRoot 'target/release-with-debug'
$binary = Join-Path $binaryDirectory 'gitcomet.exe'
$symbols = Join-Path $binaryDirectory 'gitcomet.pdb'
$samply = Get-Command samply -CommandType Application -ErrorAction SilentlyContinue |
    Select-Object -First 1
if ($null -eq $samply) {
    throw 'samply was not found on PATH. Install it with: cargo install --locked samply'
}
$xperf = Get-Command xperf -CommandType Application -ErrorAction SilentlyContinue
$xperfDefault = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10/Windows Performance Toolkit/xperf.exe'
if ($null -eq $xperf -and -not (Test-Path -LiteralPath $xperfDefault -PathType Leaf)) {
    throw 'Install the Windows Performance Toolkit component of the Windows ADK; samply needs xperf.exe.'
}
foreach ($file in @($binary, $symbols)) {
    if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
        throw "Missing $file. Finish: cargo build -p gitcomet --bin gitcomet --profile release-with-debug"
    }
}
if (Get-Process -Name gitcomet -ErrorAction SilentlyContinue) {
    throw 'Close existing GitComet windows before recording a new startup.'
}
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $repoRoot ("target/samply-startup-" + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
}
$captureRoot = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($OutputDirectory)
if (Test-Path -LiteralPath $captureRoot) {
    throw "Choose a new output directory; $captureRoot already exists."
}
$symbolDirectory = Join-Path $captureRoot 'symbols'
New-Item -ItemType Directory -Path $symbolDirectory | Out-Null
# Preserve the matching executable/PDB for viewing after a later rebuild.
Copy-Item -LiteralPath $binary, $symbols -Destination $symbolDirectory
$profilePath = Join-Path $captureRoot 'startup.json.gz'

Write-Host "Capture directory: $captureRoot"
Write-Host 'Allow the administrator prompt. Wait for "Profiling all processes...".'
Write-Host 'Then run this in a second PowerShell window:'
$launchScript = Join-Path $PSScriptRoot 'capture-gitcomet-startup.ps1'
$stageDirectory = Join-Path $captureRoot 'stages'
Write-Host ("& '{0}' -Binary '{1}' -OutputDirectory '{2}'" -f $launchScript.Replace("'", "''"), (Join-Path $symbolDirectory 'gitcomet.exe').Replace("'", "''"), $stageDirectory.Replace("'", "''"))
Write-Host 'Wait for repository status and history to be usable, then press Ctrl+C ONCE here.'
Write-Host "Avoid builds during capture. This $Rate Hz diagnostic capture changes measured startup timing."

$recordArguments = @(
    'record', '-a',
    '--rate', "$Rate",
    '--per-cpu-threads',
    '--cswitch-markers',
    '--gfx',
    '--unknown-event-markers',
    '--keep-etl',
    '--save-only',
    '--profile-name', 'GitComet startup',
    '-o', $profilePath
)
& $samply.Source @recordArguments
if ($LASTEXITCODE -ne 0) {
    throw "samply exited with code $LASTEXITCODE. Check its output for capture errors."
}
if (-not (Test-Path -LiteralPath $profilePath -PathType Leaf)) {
    throw "samply did not create $profilePath."
}
Write-Host "Saved profile: $profilePath"
Write-Host 'Run view-gitcomet-startup.ps1 to open the latest completed capture.'
