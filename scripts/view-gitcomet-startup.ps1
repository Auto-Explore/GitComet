<#
.SYNOPSIS
Open a recorded GitComet startup profile in samply.
.DESCRIPTION
With no arguments, opens the newest completed target/samply-startup-* capture.
Uses its saved executable/PDB when available and Microsoft's symbol server for
Windows libraries. Keep this terminal open while using the profiler viewer.
.EXAMPLE
.\scripts\view-gitcomet-startup.ps1
.EXAMPLE
.\scripts\view-gitcomet-startup.ps1 -Profile C:\captures\startup.json.gz
#>
[CmdletBinding()]
param(
    [string]$Profile = '',
    [string]$SymbolDirectory = ''
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$samply = Get-Command samply -CommandType Application -ErrorAction SilentlyContinue |
    Select-Object -First 1
if ($null -eq $samply) {
    throw 'samply was not found on PATH.'
}
if ([string]::IsNullOrWhiteSpace($Profile)) {
    $targetDirectory = Join-Path $repoRoot 'target'
    if (Test-Path -LiteralPath $targetDirectory -PathType Container) {
        $captures = Get-ChildItem -LiteralPath $targetDirectory -Directory -Filter 'samply-startup-*' |
            Sort-Object Name -Descending
        foreach ($capture in $captures) {
            $candidate = Join-Path $capture.FullName 'startup.json.gz'
            if ((Test-Path -LiteralPath $candidate -PathType Leaf) -and
                (Get-Item -LiteralPath $candidate).Length -gt 0) {
                $Profile = $candidate
                break
            }
        }
    }
    if ([string]::IsNullOrWhiteSpace($Profile)) {
        throw 'No completed startup capture found. Run record-gitcomet-startup.ps1, or supply -Profile.'
    }
}
$profilePath = (Resolve-Path -LiteralPath $Profile).Path
if ([string]::IsNullOrWhiteSpace($SymbolDirectory)) {
    $savedSymbols = Join-Path (Split-Path -Parent $profilePath) 'symbols'
    $SymbolDirectory = if (Test-Path -LiteralPath $savedSymbols -PathType Container) {
        $savedSymbols
    } else {
        Join-Path $repoRoot 'target/release-with-debug'
    }
}
$symbolPath = (Resolve-Path -LiteralPath $SymbolDirectory).Path
Write-Host "Opening $profilePath"
Write-Host 'Select GitComet and the startup interval, then inspect Call Tree, Flame Graph, and Stack Chart.'
& $samply.Source load --verbose --symbol-dir $symbolPath `
    --windows-symbol-server https://msdl.microsoft.com/download/symbols $profilePath
if ($LASTEXITCODE -ne 0) {
    throw "samply exited with code $LASTEXITCODE."
}
