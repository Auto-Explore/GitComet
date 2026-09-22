param(
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][string]$Repository,
    [Parameter(Mandatory)][string]$Binary,
    [ValidateSet('idle','move','resize','native-move','native-resize','hover','scroll','click','typing')]
    [string[]]$Scenarios = @('idle','move','resize','hover','scroll','click'),
    [ValidateRange(2,120)][int]$SecondsPerScenario = 5,
    [ValidateRange(2,120)][int]$WarmupSeconds = 6,
    [ValidateRange(1,240)][int]$InputRate = 60,
    [ValidateSet('auto','on','off')][string]$D3DValidation = 'auto',
    [switch]$NativeGestures,
    [switch]$CaptureScreenshots,
    [switch]$NoProbe
)
$ErrorActionPreference = 'Stop'
if (($CaptureScreenshots -or $Scenarios -contains 'native-move' -or $Scenarios -contains 'native-resize' -or $Scenarios -contains 'typing') -and -not $NativeGestures) {
    throw 'Native scenarios temporarily control the pointer/focus; pass -NativeGestures on an idle desktop.'
}
$Binary = (Resolve-Path -LiteralPath $Binary).Path
$Repository = (Resolve-Path -LiteralPath $Repository).Path
$harnessPs1Hash = (Get-FileHash -LiteralPath $PSCommandPath).Hash
$harnessCsHash = (Get-FileHash -LiteralPath (Join-Path $PSScriptRoot 'windows/ui-responsiveness.cs')).Hash
$outputDir = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $outputDir) { throw "Use a new output directory: $outputDir" }
New-Item -ItemType Directory -Path $outputDir | Out-Null
. ([ScriptBlock]::Create((Get-Content -Raw (Join-Path $PSScriptRoot 'measure-process-tree.ps1'))))
if (-not ('GitCometUiScenario' -as [type])) { Add-Type -Path (Join-Path $PSScriptRoot 'windows/ui-responsiveness.cs') -ReferencedAssemblies System.Drawing }
[void][GitCometUiScenario]::SetProcessDPIAware()
$environmentBefore = @{}
foreach ($name in @('LOCALAPPDATA','GITCOMET_SESSION_FILE','GITCOMET_DISABLE_SESSION_PERSIST','GITCOMET_UI_PROBE','GITCOMET_UI_PROBE_LOG','GITCOMET_UI_PROBE_JSONL','GPUI_D3D_DEBUG')) {
    $environmentBefore[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
$savedCursor = [GitCometUiScenario+Point]::new()
[void][GitCometUiScenario]::GetCursorPos([ref]$savedCursor)
$savedForeground = [GitCometUiScenario]::GetForegroundWindow()
$phases = [Collections.Generic.List[object]]::new()
$job = [IntPtr]::Zero; $monitor = $null; $deadline = $null; $outcome = 'failed'; $nativeActive = $false
$modules = @(); $targetPid = 0; $timerActive = $false
$gpu = @(Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,CurrentRefreshRate,CurrentHorizontalResolution,CurrentVerticalResolution)
try {
    $timerActive = [GitCometUiScenario]::timeBeginPeriod(1) -eq 0
    if (-not $timerActive) { throw 'Cannot establish controller timer resolution' }
    $env:LOCALAPPDATA = Join-Path $outputDir 'appdata'
    New-Item -ItemType Directory -Path $env:LOCALAPPDATA | Out-Null
    $env:GITCOMET_SESSION_FILE = Join-Path $outputDir 'session.json'
    $env:GITCOMET_DISABLE_SESSION_PERSIST = '1'
    $env:GITCOMET_UI_PROBE = if ($NoProbe) { '0' } else { '1' }
    $env:GITCOMET_UI_PROBE_LOG = Join-Path $outputDir 'ui.log'
    $env:GITCOMET_UI_PROBE_JSONL = Join-Path $outputDir 'frames.jsonl'
    $env:GPUI_D3D_DEBUG = $D3DValidation
    $sessionJson = @{
        version=3;open_repos=@($Repository);active_repo=$Repository
        window_width=1200;window_height=800;ui_scale_percent=100
        history_verify_commit_signatures=$false;history_verify_commit_signatures_opt_in=$false
        history_tag_fetch_mode='disabled';check_for_updates_on_startup=$false
    } | ConvertTo-Json
    [IO.File]::WriteAllText($env:GITCOMET_SESSION_FILE, $sessionJson, [Text.UTF8Encoding]::new($false))
    $job = [GitCometBenchmarkJob]::Start($Binary,('"' + $Repository + '"'),[ref]$targetPid)
    # A synchronous Windows positioning call can wait on a hung target. The
    # watchdog owns only this job and unblocks those calls by ending the app.
    $deadline = [GitCometUiScenario+JobDeadline]::new($job, (60 + $WarmupSeconds + $Scenarios.Count * ($SecondsPerScenario + 3)))
    $app = Get-Process -Id $targetPid
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $window = [IntPtr]::Zero
    while ($clock.Elapsed.TotalSeconds -lt 20 -and $window -eq 0) {
        $app.Refresh(); if ($app.HasExited) { throw 'Application exited during startup' }
        $window = [GitCometUiScenario]::Find($targetPid)
        Start-Sleep -Milliseconds 100
    }
    if ($window -eq 0) { throw 'No application window' }
    [void][GitCometUiScenario]::ShowWindow($window,4)
    [void][GitCometUiScenario]::SetWindowPos($window,[IntPtr]::Zero,120,90,1200,800,0x14)
    $monitor = [GitCometUiScenario+MoveSizeMonitor]::new([uint32]$targetPid)
    Start-Sleep -Seconds $WarmupSeconds
    if ($CaptureScreenshots) {
        [void][GitCometUiScenario]::SetForegroundWindow($window)
        Start-Sleep -Milliseconds 300
        [GitCometUiScenario]::Capture($window, (Join-Path $outputDir 'initial.png'))
    }
    $modules = @($app.Modules | Where-Object ModuleName -Match 'SDKLayers|dxgidebug' | ForEach-Object ModuleName)
    foreach ($phase in $Scenarios) {
        [void][GitCometUiScenario]::SetWindowPos($window,[IntPtr]::Zero,120,90,1200,800,0x14)
        Start-Sleep -Milliseconds 1200
        $startsBefore = $monitor.Starts; $endsBefore = $monitor.Ends
        $boundsBefore = [GitCometUiScenario+Rect]::new()
        [void][GitCometUiScenario]::GetWindowRect($window,[ref]$boundsBefore)
        if ($phase -like 'native-*') {
            $nativeActive = $true
            $anchorX = if ($phase -eq 'native-move') { $boundsBefore.Left + 600 } else { $boundsBefore.Right - 2 }
            $anchorY = if ($phase -eq 'native-move') { $boundsBefore.Top + 16 } else { $boundsBefore.Bottom - 2 }
            [GitCometUiScenario]::BeginGesture($window,$anchorX,$anchorY)
            Start-Sleep -Milliseconds 100
        }
        if ($phase -eq 'typing') { [GitCometUiScenario]::FocusBranchFilter($window); Start-Sleep -Milliseconds 300 }
        $started = $clock.Elapsed.TotalSeconds; $startedUtc = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        $app.Refresh(); $cpuBefore = $app.TotalProcessorTime.TotalSeconds
        $api = [Collections.Generic.List[double]]::new(); $actions = 0; $next = $started
        while ($clock.Elapsed.TotalSeconds - $started -lt $SecondsPerScenario) {
            $now = $clock.Elapsed.TotalSeconds
            if ($phase -ne 'idle' -and $now -ge $next) {
                $operation = [Diagnostics.Stopwatch]::StartNew()
                switch ($phase) {
                    'move' { [void][GitCometUiScenario]::SetWindowPos($window,[IntPtr]::Zero,(120+[int](70*[Math]::Sin($actions/20.0))),(90+[int](40*[Math]::Cos($actions/20.0))),0,0,0x15) }
                    'resize' { [void][GitCometUiScenario]::SetWindowPos($window,[IntPtr]::Zero,0,0,(1120+[int](70*[Math]::Sin($actions/20.0))),(740+[int](40*[Math]::Cos($actions/20.0))),0x16) }
                    { $_ -like 'native-*' } { [void][GitCometUiScenario]::SetCursorPos(($anchorX+[int](70*[Math]::Sin($actions/20.0))),($anchorY+[int](40*[Math]::Sin($actions/25.0)))) }
                    'hover' { [void][GitCometUiScenario]::PostMessage($window,0x200,[IntPtr]::Zero,[GitCometUiScenario]::XY((40+($actions*9)%950),52)) }
                    'scroll' { $delta = if ([int][Math]::Floor($actions/45.0)%2 -eq 0) { -120 } else { 120 }; [GitCometUiScenario]::Wheel($window,$delta,500,250) }
                    'click' {
                        $x = if ($actions%2 -eq 0) { 100 } else { 40 }
                        [void][GitCometUiScenario]::PostMessage($window,0x201,[IntPtr]1,[GitCometUiScenario]::XY($x,85))
                        [void][GitCometUiScenario]::PostMessage($window,0x202,[IntPtr]::Zero,[GitCometUiScenario]::XY($x,85))
                    }
                    'typing' {
                        if ($actions%8 -eq 7) { [GitCometUiScenario]::Backspace($window) }
                        else { [void][GitCometUiScenario]::PostMessage($window,0x102,[IntPtr](97+($actions%7)),[IntPtr]1) }
                    }
                }
                $api.Add($operation.Elapsed.TotalMilliseconds); $actions++
                $period = if ($phase -eq 'click') { 0.3 } elseif ($phase -eq 'typing') { 0.1 } else { 1.0/$InputRate }
                # Do not deliver a catch-up burst when the target was blocked.
                $next = [Math]::Max(($next+$period),$clock.Elapsed.TotalSeconds)
            }
            Start-Sleep -Milliseconds 1
        }
        $endedUtc = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        $app.Refresh(); $cpu = $app.TotalProcessorTime.TotalSeconds-$cpuBefore
        $boundsAfter = [GitCometUiScenario+Rect]::new()
        [void][GitCometUiScenario]::GetWindowRect($window,[ref]$boundsAfter)
        if ($nativeActive) { [GitCometUiScenario]::MouseUp(); $nativeActive=$false; Start-Sleep -Milliseconds 150 }
        $entered = $monitor.Starts-$startsBefore; $exited = $monitor.Ends-$endsBefore
        $valid = $phase -notlike 'native-*' -or ($entered -ge 1 -and $exited -ge 1 -and ($boundsBefore | ConvertTo-Json -Compress) -ne ($boundsAfter | ConvertTo-Json -Compress))
        $phases.Add([pscustomobject]@{name=$phase;start_unix_ms=$startedUtc;end_unix_ms=$endedUtc;seconds=($endedUtc-$startedUtc)/1000.0;actions=$actions;cpu_seconds=$cpu;api_ms=@($api.ToArray());native_starts=$entered;native_ends=$exited;valid=$valid;bounds_before=$boundsBefore;bounds_after=$boundsAfter})
        Write-Output ("{0}: {1} actions, CPU {2:N3}s, valid={3}" -f $phase,$actions,$cpu,$valid)
        if (-not $valid) { throw "Native scenario did not complete a verified move/resize: $phase" }
        if ($CaptureScreenshots) { [GitCometUiScenario]::Capture($window, (Join-Path $outputDir ($phase + '.png'))) }
        if ($phase -eq 'typing') { [void][GitCometUiScenario]::PostMessage($window,0x100,[IntPtr]27,[IntPtr]1) }
    }
    # Drain the final probe interval before shutdown.
    Start-Sleep -Milliseconds 1300
    [void][GitCometUiScenario]::PostMessage($window,0x10,[IntPtr]::Zero,[IntPtr]::Zero)
    if (-not $app.WaitForExit(5000)) { throw 'Application did not close normally' }
    $exitCode = [GitCometBenchmarkJob]::ExitCodeAndRelease($targetPid)
    if ($exitCode -ne 0) { throw "Application exited $exitCode" }
    $outcome = 'passed'
} finally {
    if ($nativeActive) { [GitCometUiScenario]::MouseUp() }
    if ($monitor) { $monitor.Dispose() }
    if ($deadline) { $deadline.Dispose() }
    if ($job -ne [IntPtr]::Zero) { [void][GitCometBenchmarkJob]::TerminateJobObject($job,1); [void][GitCometBenchmarkJob]::CloseHandle($job) }
    if ($NativeGestures) { [void][GitCometUiScenario]::SetCursorPos($savedCursor.X,$savedCursor.Y); [void][GitCometUiScenario]::SetForegroundWindow($savedForeground) }
    if ($timerActive) { [void][GitCometUiScenario]::timeEndPeriod(1) }
    foreach ($name in $environmentBefore.Keys) { [Environment]::SetEnvironmentVariable($name,$environmentBefore[$name],'Process') }
    if ($harnessPs1Hash -ne (Get-FileHash -LiteralPath $PSCommandPath).Hash -or $harnessCsHash -ne (Get-FileHash -LiteralPath (Join-Path $PSScriptRoot 'windows/ui-responsiveness.cs')).Hash) { $outcome = 'harness-changed' }
    $captureJson = @{version=1;outcome=$outcome;binary=$Binary;sha256=(Get-FileHash -LiteralPath $Binary).Hash.ToLowerInvariant();repository=$Repository;repository_sha=(git -C $Repository rev-parse HEAD);input_rate=$InputRate;probe=(-not $NoProbe);d3d_validation=$D3DValidation;debug_modules=$modules;gpu=$gpu;phases=@($phases.ToArray());harness_sha=(git -C (Join-Path $PSScriptRoot '..') rev-parse HEAD);harness_ps1_sha256=$harnessPs1Hash;harness_cs_sha256=$harnessCsHash} | ConvertTo-Json -Depth 8
    [IO.File]::WriteAllText((Join-Path $outputDir 'capture.json'), $captureJson, [Text.UTF8Encoding]::new($false))
}
