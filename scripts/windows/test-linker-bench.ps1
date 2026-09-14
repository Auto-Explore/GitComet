#requires -Version 7.0
[CmdletBinding()]
param([switch]$SkipBuild, [switch]$BuildScenarios)
. (Join-Path $PSScriptRoot 'linker-bench-common.ps1')
$repo=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$toolTarget=Join-Path $repo 'target/linker-bench-tools'
if (!$SkipBuild) {
    & cargo test --locked --offline --manifest-path (Join-Path $PSScriptRoot 'linker-metrics/Cargo.toml') --target-dir $toolTarget
    if ($LASTEXITCODE -ne 0) { throw 'Rust unit tests failed' }
    & cargo build --release --locked --offline --manifest-path (Join-Path $PSScriptRoot 'linker-metrics/Cargo.toml') --target-dir $toolTarget
    if ($LASTEXITCODE -ne 0) { throw 'Building test helper failed' }
}
$helper=Join-Path $toolTarget 'release/gitcomet-linker-metrics.exe'
$testRoot=Join-Path $toolTarget (New-BenchId 'tests')
New-Item -ItemType Directory -Path $testRoot -Force | Out-Null
function Assert-That([bool]$Condition,[string]$Message) { if (!$Condition) { throw $Message } }
function Run-TestProcess([string]$Name,[string[]]$Arguments,[int]$Timeout=20,[string]$Executable=$helper) {
    $output=Join-Path $testRoot $Name
    New-Item -ItemType Directory -Path $output -Force | Out-Null
    $spec=Join-Path $output 'spec.json'
    Write-BenchJson $spec @{executable=$Executable;arguments=$Arguments;cwd=$output;output_dir=$output;sample_ms=20;timeout_s=$Timeout;metadata=@{scope='test'}}
    & $helper --run-spec $spec
    return Read-BenchJson (Join-Path $output 'metrics.json')
}

$workload=Run-TestProcess accounting @('--self-test-workload','child')
Assert-That ($workload.valid -and $workload.exit_code -eq 0) 'Workload failed'
Assert-That ($workload.tree_process_count -ge 2) 'Child was missing from job accounting'
Assert-That ($workload.peak_tree_commit_bytes -gt 64MB) 'Memory peak missed workload allocations'
Assert-That ($workload.tree_io.read_bytes -ge 64MB -and $workload.tree_io.write_bytes -ge 64MB) 'Child I/O was missing'
Assert-That ($workload.tree_io.read_bytes -ge $workload.root_io.read_bytes) 'Tree I/O smaller than root I/O'
Assert-That ($workload.sample_count -gt 0) 'Memory sampling failed'
Assert-That ($workload.wall_s -ge $workload.process_lifetime_s -and ($workload.wall_s-$workload.process_lifetime_s) -lt 0.2) 'Exit waiter timestamp differs excessively from OS process lifetime'
$argsToEcho=@('', 'contains spaces', 'embedded"quote', 'C:\path with space\', 'before\"after', 'åäö 🦀')
$quoted=Run-TestProcess quoting (@('--echo-args')+$argsToEcho)
$echoed=@(Get-Content -LiteralPath (Join-Path $quoted.output_dir 'stdout.log') -Raw | ConvertFrom-Json)
Assert-That (($echoed | ConvertTo-Json -Compress) -ceq ($argsToEcho | ConvertTo-Json -Compress)) 'Native Windows command-line quoting changed arguments'
$failed=Run-TestProcess exit @('--exit-code','37')
Assert-That ($failed.valid -and $failed.exit_code -eq 37) 'Child exit status was not preserved'
$missing=Run-TestProcess missing @() 20 (Join-Path $testRoot 'does-not-exist.exe')
Assert-That (!$missing.valid -and $missing.exit_code -eq 125 -and $missing.collector_error) 'Launch failure was not retained'
$timeout=Run-TestProcess timeout @('--sleep-ms','5000') 1
Assert-That (!$timeout.valid -and $timeout.timed_out -and $timeout.exit_code -eq 124) 'Timeout was not recorded'
foreach ($bad in @($testRoot,(Join-Path $testRoot '..'))) {
    $rejected=$false
    try { $null=Assert-BenchDescendant $bad $testRoot } catch { $rejected=$true }
    Assert-That $rejected 'Cleanup path guard allowed root/parent deletion'
}
$order=@(Get-BenchPairOrder 10 7)
Assert-That (@($order | Where-Object { $_[0] -eq 'lld' }).Count -eq 5) 'Run ordering is not balanced'

# Exercise long input paths, same-named libraries, and search-order preservation.
$fixtureRoot=Join-Path $testRoot 'original'
$longDirectory=Join-Path $fixtureRoot (('long-directory/' * 16) + 'one')
$otherDirectory=Join-Path $fixtureRoot 'two'
New-Item -ItemType Directory -Path $longDirectory,$otherDirectory -Force | Out-Null
[IO.File]::WriteAllText((Join-Path $longDirectory 'same.lib'),'first')
[IO.File]::WriteAllText((Join-Path $otherDirectory 'same.lib'),'second')
$longRelative=[IO.Path]::GetRelativePath($fixtureRoot,$longDirectory)
$inputFixture=@{profile='test';cwd=$fixtureRoot;fixture_sha256='test-inputs';arguments=@("/libpath:$longRelative",'/libpath:two',(Join-Path $longRelative 'same.lib'),'two/same.lib','/OPT:REF')}
$compact=Get-BenchCompactFixture $inputFixture $testRoot
Assert-That ($compact.arguments[2].Length -lt 260) 'Compact input still exceeds LINK path limit'
Assert-That ([IO.File]::ReadAllText($compact.arguments[2]) -eq 'first' -and [IO.File]::ReadAllText($compact.arguments[3]) -eq 'second') 'Compact layout lost distinct same-named libraries'
Assert-That ($compact.arguments[0] -eq ('/libpath:'+[IO.Path]::GetDirectoryName($compact.arguments[2])) -and $compact.arguments[1] -eq ('/libpath:'+[IO.Path]::GetDirectoryName($compact.arguments[3]))) 'Compact layout changed library search order'
Assert-That ((Get-BenchCompactFixture $inputFixture $testRoot).fixture_sha256 -eq $compact.fixture_sha256) 'Compact layout identity is unstable'
[IO.File]::WriteAllText($compact.arguments[2],'corrupted')
$rejected=$false
try { $null=Get-BenchCompactFixture $inputFixture $testRoot } catch { $rejected=$true }
Assert-That $rejected 'Compact input corruption went undetected'

# End-to-end aggregation test: a failed outlier and an unpaired run must not
# alter the 50% paired reduction or silently become successful observations.
$summaryRoot=Join-Path $testRoot 'summary'
New-Item -ItemType Directory -Path $summaryRoot -Force | Out-Null
Write-BenchJson (Join-Path $summaryRoot 'manifest.json') @{campaign_id='synthetic-test';seed=7}
for ($pair=0; $pair -lt 3; $pair++) {
    foreach ($backend in @('msvc','lld')) {
        $directory=Join-Path $summaryRoot "runs/$pair-$backend"
        New-Item -ItemType Directory -Path $directory -Force | Out-Null
        $metadata=@{run_id="$pair-$backend";pair_id="pair-$pair";profile='dev';scenario='replay';scope='linker';linker=$backend;
            fixture_sha256='same';source_sha256='same';cache_policy='warm';output_state='fresh';diagnostic_mode='none';lld_threads=0;warmup=$false}
        Write-BenchJson (Join-Path $directory 'metrics.json') @{schema_version=1;metadata=$metadata;valid=$true;exit_code=0;sample_ms=50;wall_s=$(if($backend -eq 'lld'){1.0}else{2.0})}
    }
}
$failureDir=Join-Path $summaryRoot 'runs/failed'
New-Item -ItemType Directory -Path $failureDir -Force | Out-Null
$outlier=Read-BenchJson (Join-Path $summaryRoot 'runs/0-msvc/metrics.json')
$outlier.exit_code=1; $outlier.wall_s=100; $outlier.metadata.pair_id='unpaired-failure'
Write-BenchJson (Join-Path $failureDir 'metrics.json') $outlier
& (Join-Path $PSScriptRoot 'summarize-linkers.ps1') -Campaign $summaryRoot -BootstrapSamples 200
$comparison=@(Read-BenchJson (Join-Path $summaryRoot 'comparison.json'))
Assert-That ($comparison.Count -eq 1 -and $comparison[0].pairs -eq 3 -and $comparison[0].paired_reduction_percent -eq 50 -and $comparison[0].ci95_low -eq 50 -and $comparison[0].ci95_high -eq 50) 'Paired aggregation/bootstrap failed'
$summary=Import-Csv -LiteralPath (Join-Path $summaryRoot 'summary.csv') | Where-Object { $_.linker -eq 'msvc' -and $_.metric -eq 'wall_s' }
Assert-That ($summary.failures -eq 1 -and $summary.n -eq 3) 'Failure accounting was lost'
if ($BuildScenarios) {
    $scenarioRoot=Join-Path $repo ('target/linker-bench/' + (New-BenchId 'scenario-test'))
    $scenarioSource=Join-Path $scenarioRoot 'source'
    New-Item -ItemType Directory -Path (Join-Path $scenarioSource 'crates/gitcomet/src') -Force | Out-Null
    [IO.File]::WriteAllText((Join-Path $scenarioSource 'Cargo.toml'), "[workspace]`nresolver = `"3`"`nmembers = [`"crates/gitcomet`"]`n")
    [IO.File]::WriteAllText((Join-Path $scenarioSource 'crates/gitcomet/Cargo.toml'), "[package]`nname = `"gitcomet`"`nversion = `"0.1.0`"`nedition = `"2024`"`n")
    [IO.File]::WriteAllText((Join-Path $scenarioSource 'crates/gitcomet/src/main.rs'), "fn main() { println!(`"fixture`"); }`n")
    & cargo generate-lockfile --offline --manifest-path (Join-Path $scenarioSource 'Cargo.toml')
    if ($LASTEXITCODE -ne 0) { throw 'Fixture lockfile generation failed' }
    $scenarioManifest=@{schema_version=1;campaign_id=(Split-Path $scenarioRoot -Leaf);campaign=$scenarioRoot;source=$scenarioSource;
        source_sha256=(Get-BenchHash (Join-Path $scenarioSource 'crates/gitcomet/src/main.rs'));helper=$helper;helper_sha256=(Get-BenchHash $helper);
        tools=(Get-BenchToolchain);seed=7;cargo_jobs=2}
    Write-BenchJson (Join-Path $scenarioRoot 'manifest.json') $scenarioManifest
    & (Join-Path $PSScriptRoot 'benchmark-linkers.ps1') -Stage Build -Campaign $scenarioRoot -Profiles dev -BuildScenarios edit,clean,noop -BuildPairs 1
    $scenarioRows=Import-Csv -LiteralPath (Join-Path $scenarioRoot 'runs.csv')
    Assert-That (@($scenarioRows | Where-Object { $_.scope -eq 'cargo' -and $_.scenario -eq 'build-edit' -and $_.exit_code -eq 0 }).Count -eq 2) 'Both edit builds were not measured'
    Assert-That (@($scenarioRows | Where-Object { $_.scope -eq 'cargo' -and $_.scenario -eq 'build-clean' -and $_.exit_code -eq 0 }).Count -eq 2) 'Both clean builds were not measured'
    Assert-That (@($scenarioRows | Where-Object { $_.scope -eq 'cargo' -and $_.scenario -eq 'build-noop' -and $_.exit_code -eq 0 }).Count -eq 2) 'Both no-op builds were not measured'
}
Write-Host "All linker benchmark tests passed. Artifacts: $testRoot"
