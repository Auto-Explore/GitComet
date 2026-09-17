param(
    [Parameter(Mandatory)][string]$FixtureRoot,
    [Parameter(Mandatory)][string]$OutputFile,
    [string]$Binary = (Join-Path $PSScriptRoot '../target/release-with-debug/examples/status-refresh-bench.exe'),
    [int]$Rounds = 5,
    [string[]]$Workers = @('auto', '1', '4', '8', 'production')
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'measure-process-tree.ps1')
$Binary = (Resolve-Path -LiteralPath $Binary).Path
$FixtureRoot = [IO.Path]::GetFullPath($FixtureRoot)
$fixtures = @('plain', 'small-lfs', 'dirty-lfs', 'mixed', 'large-lfs', 'many-lfs')
New-Item -ItemType Directory -Path $FixtureRoot -Force | Out-Null
$oldGlobal = $env:GIT_CONFIG_GLOBAL
$oldSystem = $env:GIT_CONFIG_NOSYSTEM
$oldWorkers = $env:GITCOMET_BENCH_STATUS_WORKERS
try {
    $env:GIT_CONFIG_GLOBAL = Join-Path $FixtureRoot 'empty.gitconfig'
    $env:GIT_CONFIG_NOSYSTEM = '1'
    Set-Content -LiteralPath $env:GIT_CONFIG_GLOBAL -Value ''
    foreach ($kind in $fixtures) {
        $path = Join-Path $FixtureRoot $kind
        if (-not (Test-Path -LiteralPath $path)) {
            & $Binary init $path $kind
            if ($LASTEXITCODE -ne 0) { throw "Could not create $kind" }
        }
        if (-not (Test-Path -LiteralPath (Join-Path $path '.git/gitcomet-benchmark-fixture'))) { throw "Not a disposable fixture: $path" }
    }
    $results = [Collections.Generic.List[object]]::new()
    for ($round = 0; $round -lt $Rounds; $round++) {
        $order = @($Workers)
        if ($round % 2 -ne 0) { [array]::Reverse($order) }
        foreach ($kind in $fixtures) {
            foreach ($workerMode in $order) {
                $env:GITCOMET_BENCH_STATUS_WORKERS = $workerMode
                $path = Join-Path $FixtureRoot $kind
                $sample = Measure-GitCometProcessTree -Binary $Binary -Arguments ('run "' + $path + '" 1')
                $row = [pscustomobject]@{ round = $round; fixture = $kind; workers = $workerMode; wall_seconds = $sample.wall_seconds; tree_cpu_seconds = $sample.tree_cpu_seconds; processes = $sample.processes }
                $results.Add($row)
                $row | ConvertTo-Json -Compress | Write-Output
                $results | ConvertTo-Json | Set-Content -LiteralPath $OutputFile
            }
        }
    }
} finally {
    $env:GIT_CONFIG_GLOBAL = $oldGlobal
    $env:GIT_CONFIG_NOSYSTEM = $oldSystem
    $env:GITCOMET_BENCH_STATUS_WORKERS = $oldWorkers
}
