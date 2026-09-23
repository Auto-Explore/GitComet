param(
    [Parameter(Mandatory)][string]$OutputDirectory,
    [Parameter(Mandatory)][string]$Repository,
    [string]$Binary = (Join-Path $PSScriptRoot '../../target/release-with-debug/gitcomet.exe'),
    [ValidateSet('idle','scroll','activate')][string]$Scenario = 'idle',
    [ValidateRange(1, 86400)][int]$Seconds = 45,
    [switch]$VerifySignatures,
    [switch]$TraceGit,
    [switch]$NoProbe
)
$NoSignatures = -not $VerifySignatures
$Label = Split-Path -Leaf $OutputDirectory
$Binary = (Resolve-Path -LiteralPath $Binary).Path
$Repository = (Resolve-Path -LiteralPath $Repository).Path
. (Join-Path $PSScriptRoot 'measure-process-tree.ps1')
$ErrorActionPreference = 'Stop'
$outputDir = [IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Path $outputDir -ErrorAction Stop | Out-Null
$environmentBefore = @{}
foreach ($name in @('GITCOMET_SESSION_FILE','GITCOMET_DISABLE_SESSION_PERSIST','GITCOMET_UI_PROBE','GITCOMET_UI_PROBE_LOG','GITCOMET_REPO_LOAD_TRACE','GIT_TRACE2_EVENT','LOCALAPPDATA')) {
    $environmentBefore[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
try {
$session = @{
    version = 3; open_repos = @($Repository); active_repo = $Repository
    window_width = 1280; window_height = 900; ui_scale_percent = 100
    history_verify_commit_signatures = -not $NoSignatures
    history_verify_commit_signatures_opt_in = -not $NoSignatures
    history_tag_fetch_mode = 'disabled'; check_for_updates_on_startup = $false
}
$session | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $outputDir 'session.json')
$env:GITCOMET_SESSION_FILE = Join-Path $outputDir 'session.json'
$env:GITCOMET_DISABLE_SESSION_PERSIST = '1'
$env:GITCOMET_UI_PROBE = if ($NoProbe) { '0' } else { '1' }
$env:GITCOMET_UI_PROBE_LOG = Join-Path $outputDir 'ui.log'
if ($TraceGit) { $env:GIT_TRACE2_EVENT = Join-Path $outputDir 'git-trace2.json' }
if ($NoProbe) { Remove-Item Env:GITCOMET_REPO_LOAD_TRACE -ErrorAction SilentlyContinue }
else { $env:GITCOMET_REPO_LOAD_TRACE = Join-Path $outputDir 'loads.log' }
$env:LOCALAPPDATA = Join-Path $outputDir 'appdata'
New-Item -ItemType Directory -Path $env:LOCALAPPDATA | Out-Null
if (-not ('GitCometBenchmarkWindow' -as [type])) {
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class GitCometBenchmarkWindow {
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out Rect r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref Point p);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h,int command);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h,out uint pid);
    delegate bool EnumProc(IntPtr h,IntPtr param);
    [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback,IntPtr param);
    public static IntPtr Find(int pid) {
        IntPtr found=IntPtr.Zero;
        EnumWindows((h,p)=> { uint owner; GetWindowThreadProcessId(h,out owner); Rect rect; GetClientRect(h,out rect); if(owner==(uint)pid && rect.Right>=400 && rect.Bottom>=250) { found=h; return false; } return true; },IntPtr.Zero);
        return found;
    }
    public static IntPtr XY(int x,int y) { return new IntPtr((y << 16) | (x & 65535)); }
    public static void Wheel(IntPtr h,int delta,int x,int y) {
        PostMessage(h,0x200,IntPtr.Zero,XY(x,y));
        Point p = new Point { X=x,Y=y }; ClientToScreen(h,ref p);
        PostMessage(h,0x20a,new IntPtr(delta << 16),XY(p.X,p.Y));
    }
}

'@
}
$targetArgs = @($Repository)
$appArgs = ($targetArgs | ForEach-Object { '"' + $_ + '"' }) -join ' '
    $targetPid = 0
    $jobHandle = [GitCometBenchmarkJob]::Start($Binary,$appArgs,[ref]$targetPid)
    $capture = Get-Process -Id $targetPid
    $app = $capture
$samples = [Collections.Generic.List[object]]::new()
$events = [Collections.Generic.List[object]]::new()
$outcome = 'unknown'
try {
    $clock = [Diagnostics.Stopwatch]::StartNew()
    $nextSample = 0.0; $nextAction = 15.0; $actionCount = 0; $testWindow = [IntPtr]::Zero
    while ($clock.Elapsed.TotalSeconds -lt $Seconds) {
        $app.Refresh()
        if ($app.HasExited) { $outcome = 'early-exit'; break }
        if ($testWindow -eq 0) {
            $testWindow = [GitCometBenchmarkWindow]::Find($app.Id)
            if ($testWindow -ne 0) { [void][GitCometBenchmarkWindow]::ShowWindow($testWindow,4) }
        }
        $elapsed = $clock.Elapsed.TotalSeconds
        if ($elapsed -ge $nextSample) {
            $hwnd = $testWindow
            $jobStats = [GitCometBenchmarkJob]::Read($jobHandle)
            $samples.Add([pscustomobject]@{seconds=$elapsed; cpu_seconds=$app.TotalProcessorTime.TotalSeconds; tree_cpu_seconds=$jobStats.CpuSeconds; tree_processes=$jobStats.TotalProcesses; active_tree_processes=$jobStats.ActiveProcesses; working_set=$app.WorkingSet64; private_bytes=$app.PrivateMemorySize64; threads=$app.Threads.Count; hwnd=$hwnd.ToInt64(); visible=($hwnd -ne 0 -and [GitCometBenchmarkWindow]::IsWindowVisible($hwnd))})
            $nextSample += 1
        }
        if ($elapsed -ge $nextAction -and $testWindow -ne 0) {
            $hwnd = $testWindow
            if ($Scenario -eq 'scroll') {
                # Leave time to verify the final settled viewport after a scroll burst.
                if ($elapsed -ge ($Seconds - 3)) { $nextAction = $Seconds + 1; continue }
                $rect = [GitCometBenchmarkWindow+Rect]::new()
                [void][GitCometBenchmarkWindow]::GetClientRect($hwnd,[ref]$rect)
                $x = [int]($rect.Right * 0.55)
                $y = [int]($rect.Bottom * 0.30)
                $delta = if (([int][math]::Floor(($elapsed - 15) / 8) % 2) -eq 0) { -120 } else { 120 }
                [GitCometBenchmarkWindow]::Wheel($hwnd,$delta,$x,$y)
                $actionCount += 1
                $nextAction += 0.033333333
            } elseif ($Scenario -eq 'activate') {
                [void][GitCometBenchmarkWindow]::PostMessage($hwnd,6,[IntPtr]::Zero,[IntPtr]::Zero)
                Start-Sleep -Milliseconds 30
                [void][GitCometBenchmarkWindow]::PostMessage($hwnd,6,[IntPtr]1,[IntPtr]::Zero)
                $events.Add([pscustomobject]@{seconds=$elapsed;action='synthetic deactivate/activate'})
                $actionCount += 1
                $nextAction += 6
            } else { $nextAction = $Seconds + 1 }
        }
        Start-Sleep -Milliseconds 8
    }
    if ($outcome -eq 'unknown') { $outcome = 'completed' }
    $app.Refresh()
    if (-not $app.HasExited) {
        [void][GitCometBenchmarkWindow]::PostMessage($testWindow,0x10,[IntPtr]::Zero,[IntPtr]::Zero)
        if (-not $app.WaitForExit(5000)) { $app.Kill(); $outcome += '-forced-shutdown' }
    }
    if (-not $capture.WaitForExit(30000)) { throw 'Application did not exit' }
} finally {
    if ($app -and -not $app.HasExited) { $app.Kill() }
    if ($jobHandle -ne 0) {
        [GitCometBenchmarkJob]::Read($jobHandle) | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $outputDir 'final-tree-cpu.json')
        [void][GitCometBenchmarkJob]::TerminateJobObject($jobHandle, 1)
        [void][GitCometBenchmarkJob]::CloseHandle($jobHandle)
        $nativeExit = [GitCometBenchmarkJob]::ExitCodeAndRelease($app.Id)
    }
    $samples | Export-Csv -LiteralPath (Join-Path $outputDir 'resources.csv') -NoTypeInformation
    $events | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $outputDir 'events.json')
    @{label=$Label;scenario=$Scenario;repository=$Repository;requested_seconds=$Seconds;app_pid=if($app){$app.Id}else{$null};capture_pid=$capture.Id;actions=$actionCount;outcome=$outcome;probe=(-not $NoProbe);signatures=(-not $NoSignatures);binary=$Binary;binary_sha256=(Get-FileHash -LiteralPath $Binary).Hash;git_head=(git -C (Join-Path $PSScriptRoot '../..') rev-parse HEAD);session=$session} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $outputDir 'metadata.json')
}
if ($outcome -ne 'completed' -or $nativeExit -ne 0 -or $testWindow -eq 0) {
    throw "Invalid capture: outcome=$outcome exit=$nativeExit window=$testWindow"
}
Write-Output "$Label $outcome app_pid=$($app.Id) actions=$actionCount"

} finally {
    foreach ($name in $environmentBefore.Keys) {
        [Environment]::SetEnvironmentVariable($name, $environmentBefore[$name], 'Process')
    }
}
