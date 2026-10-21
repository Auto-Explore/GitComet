<#
.SYNOPSIS
Capture startup stages from the normal GitComet executable.
.DESCRIPTION
Close existing GitComet windows first. By default this uses your usual saved
settings and leaves the app running. -EmptyProfile uses a fresh isolated profile
to measure the repository picker. -AutoExit requests normal window closure one
second after its first interactive frame (or the ready picker with
-EmptyProfile), then waits up to five seconds for exit. If the app stays open,
it is left running. Neither switch is needed for reporter diagnostics.
.EXAMPLE
./scripts/profiling/capture-startup.ps1 -Binary ./target/release/gitcomet.exe
.EXAMPLE
./scripts/profiling/capture-startup.ps1 -Binary ./target/release/gitcomet.exe -EmptyProfile -AutoExit
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Binary,
    [string]$OutputDirectory = "",
    [switch]$EmptyProfile,
    [switch]$AutoExit,
    [ValidateRange(1, 600)][int]$TimeoutSeconds = 90
)

$ErrorActionPreference = 'Stop'
$binaryPath = (Resolve-Path -LiteralPath $Binary).Path
if ([string]::IsNullOrWhiteSpace($OutputDirectory)) {
    $OutputDirectory = Join-Path $PSScriptRoot ("../../target/startup-captures/" + (Get-Date -Format 'yyyyMMdd-HHmmss-fff'))
}
$captureRoot = [System.IO.Path]::GetFullPath($OutputDirectory)
[void][System.IO.Directory]::CreateDirectory($captureRoot)
$tracePath = Join-Path $captureRoot 'startup.jsonl'
if (Test-Path -LiteralPath $tracePath) {
    throw "A startup trace already exists in $captureRoot. Choose a fresh output directory."
}

# Change only this child's inherited environment, restoring the shell even if
# launch fails. PROFILE_ROOT also isolates the instance broker and crash data.
$changes = @{
    GITCOMET_STARTUP_TRACE = $tracePath
    GITCOMET_PERF_STARTUP_PROBE = '1'
    GITCOMET_PERF_STARTUP_AUTO_EXIT = '0'
    GITCOMET_PERF_STARTUP_EXPECT_READY_REPOS = '0'
    GITCOMET_PERF_STARTUP_WAIT_FOR_ACTIVE_REPO = '0'
    GITCOMET_PERF_STARTUP_WAIT_FOR_HOME_READY = '0'
    GITCOMET_PERF_STARTUP_DISABLE_AUTO_RESTORE = '0'
}
# Even with -AutoExit, close through the normal window lifecycle below. The
# frame probe's immediate process exit can interrupt background tool probes.
if ($EmptyProfile) {
    $changes.GITCOMET_PROFILE_ROOT = Join-Path $captureRoot 'profile'
    $changes.GITCOMET_PERF_STARTUP_DISABLE_AUTO_RESTORE = '1'
    $changes.GITCOMET_PERF_STARTUP_WAIT_FOR_HOME_READY = '1'
    # A more specific inherited override must not escape profile isolation.
    $changes.GITCOMET_SESSION_FILE = $null
    $changes.GITCOMET_BROWSER_INSTANCE_FILE = $null
}
$previous = @{}
$clock = [System.Diagnostics.Stopwatch]::StartNew()
try {
    foreach ($key in $changes.Keys) {
        $previous[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
        if ($null -eq $changes[$key]) {
            [Environment]::SetEnvironmentVariable($key, [NullString]::Value, 'Process')
        } else {
            [Environment]::SetEnvironmentVariable($key, $changes[$key], 'Process')
        }
    }
    # This is the interactive application being measured. SW_HIDE suppresses
    # frame callbacks, so the ready/interactive markers cannot be reached.
    $process = Start-Process -FilePath $binaryPath -PassThru -WindowStyle Normal `
        -RedirectStandardOutput (Join-Path $captureRoot 'stdout.log') `
        -RedirectStandardError (Join-Path $captureRoot 'stderr.log')
} finally {
    foreach ($key in $previous.Keys) {
        if ($null -eq $previous[$key]) {
            [Environment]::SetEnvironmentVariable($key, [NullString]::Value, 'Process')
        } else {
            [Environment]::SetEnvironmentVariable($key, $previous[$key], 'Process')
        }
    }
}

$targetMarker = if ($EmptyProfile) { 'home_ready' } else { 'first_interactive' }
$markerObservedMs = $null
$records = @()
while ($clock.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
    if (Test-Path -LiteralPath $tracePath) {
        # A writer can be between writes; discard an incomplete final line and
        # retry instead of treating it as a corrupt capture.
        $records = @(Get-Content -LiteralPath $tracePath | ForEach-Object {
            try { $_ | ConvertFrom-Json -ErrorAction Stop } catch { }
        } | Where-Object { $_.pid -eq $process.Id })
        if ($records | Where-Object { $_.name -eq $targetMarker }) {
            $markerObservedMs = $clock.Elapsed.TotalMilliseconds
            break
        }
    }
    $process.Refresh()
    if ($process.HasExited) { break }
    Start-Sleep -Milliseconds 100
}
$clock.Stop()
# Re-read after process exit: its last marker may have arrived between the
# previous file read and the HasExited check.
if (Test-Path -LiteralPath $tracePath) {
    $records = @(Get-Content -LiteralPath $tracePath | ForEach-Object {
        try { $_ | ConvertFrom-Json -ErrorAction Stop } catch { }
    } | Where-Object { $_.pid -eq $process.Id })
    if ($null -eq $markerObservedMs -and ($records | Where-Object { $_.name -eq $targetMarker })) {
        $markerObservedMs = $clock.Elapsed.TotalMilliseconds
    }
}
$process.Refresh()
$closeRequested = $false
$closeError = $null
if ($AutoExit -and $null -ne $markerObservedMs -and -not $process.HasExited) {
    # Keep the recorded marker time separate from this shutdown grace period.
    Start-Sleep -Milliseconds 1000
    $process.Refresh()
    if (-not $process.HasExited) {
        try {
            $closeRequested = $process.CloseMainWindow()
            if ($closeRequested) {
                [void]$process.WaitForExit(5000)
            } else {
                $closeError = 'The launched app did not accept a main-window close request.'
            }
        } catch {
            $process.Refresh()
            if (-not $process.HasExited) { $closeError = $_.Exception.Message }
        }
        $process.Refresh()
        if (-not $process.HasExited) {
            Write-Warning "The launched app is still running after the graceful-close request; it has been left open. $closeError"
        }
    }
}
$metadata = [ordered]@{
    binary = $binaryPath
    sha256 = (Get-FileHash -LiteralPath $binaryPath -Algorithm SHA256).Hash
    capturedAtUtc = [DateTime]::UtcNow.ToString('o')
    processId = $process.Id
    emptyProfile = [bool]$EmptyProfile
    autoExit = [bool]$AutoExit
    closeMode = $(if ($AutoExit) { 'main_window_close' } else { 'leave_open' })
    closeDelayMs = $(if ($AutoExit) { 1000 } else { $null })
    closeRequested = $closeRequested
    closeError = $closeError
    observedMarker = $targetMarker
    markerObservedMs = $markerObservedMs
    observationTimeoutSeconds = $TimeoutSeconds
    processExited = $process.HasExited
    exitCode = $(if ($process.HasExited) { $process.ExitCode } else { $null })
}
$metadata | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $captureRoot 'capture.json') -Encoding UTF8
$records | Where-Object { $_.kind -ne 'begin' } |
    Select-Object name, kind, @{n='elapsed_ms';e={[Math]::Round($_.elapsed_ms, 2)}},
        @{n='duration_ms';e={if ($null -ne $_.duration_ms) {[Math]::Round($_.duration_ms, 2)}}} |
    Format-Table -AutoSize
if ($null -eq $markerObservedMs) {
    Write-Warning "No $targetMarker marker was observed. Check startup.jsonl and stderr.log. Close existing GitComet windows before retrying; the executable must include startup tracing support."
    if (-not $process.HasExited) { Write-Output 'The launched app is still running; it has been left open.' }
}
Write-Output "Capture saved to $captureRoot"
