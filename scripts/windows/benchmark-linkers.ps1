#requires -Version 7.0
[CmdletBinding()]
param(
    [ValidateSet('Initialize','Capture','Replay','Calibrate','Diagnostics','Build','All')]
    [string]$Stage='All',
    [string]$Campaign,
    [ValidateSet('dev','shipping-release','release','release-with-debug')]
    [string[]]$Profiles=@('dev','shipping-release'),
    [ValidateRange(1,1000)][int]$Pairs=10,
    [ValidateRange(0,20)][int]$Warmups=2,
    [ValidateRange(0,10000)][int]$SampleMs=50,
    [ValidateRange(1,1024)][int]$Jobs=[Environment]::ProcessorCount,
    [int]$Seed=20260913,
    [ValidateRange(1,86400)][int]$TimeoutSeconds=7200,
    [ValidateSet('clean','edit','noop')][string[]]$BuildScenarios=@('edit','clean','noop'),
    [ValidateRange(1,100)][int]$BuildPairs=3,
    [ValidateRange(0,1024)][int]$LldThreads=0,
    [switch]$Etw,
    [string]$WpaProfile
)
. (Join-Path $PSScriptRoot 'linker-bench-common.ps1')
$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
if (!$Campaign) { $Campaign=Join-Path $repo ('target/linker-bench/' + (New-BenchId 'campaign')) }
$Campaign=[IO.Path]::GetFullPath($Campaign)
$null = Assert-BenchDescendant $Campaign (Join-Path $repo 'target/linker-bench')
$savedEnvironment=@{}
Get-ChildItem Env: | ForEach-Object { $savedEnvironment[$_.Name]=$_.Value }

function Complete-BenchFixture($Manifest, [string]$Profile) {
    $fixtureDir=Join-Path $Campaign "fixtures/$Profile"
    $archive=Join-Path $fixtureDir 'repro.tar'
    if (!(Test-Path -LiteralPath $archive)) { throw "No capture archive for $Profile" }
    $extract=Join-Path $fixtureDir 'inputs'
    $members=@(& tar -tf $archive)
    if ($LASTEXITCODE -ne 0) { throw 'Cannot read reproduction archive' }
    foreach ($member in $members) {
        if ([IO.Path]::IsPathRooted($member) -or $member.Split([char[]]@('/','\')).Contains('..')) { throw "Unsafe archive entry: $member" }
    }
    Reset-BenchDirectory $extract $Campaign
    & tar -xf $archive -C $extract
    if ($LASTEXITCODE -ne 0) { throw 'Could not extract reproduction archive' }
    $responses=@(Get-ChildItem -LiteralPath $extract -Filter response.txt -Recurse)
    if ($responses.Count -ne 1) { throw 'Expected exactly one captured response.txt' }
    $responsePath=$responses[0].FullName
    $cwd=$responses[0].DirectoryName
    $argsJson=& $Manifest.helper --parse-response $responsePath
    if ($LASTEXITCODE -ne 0) { throw 'Could not parse captured response file' }
    $arguments=@($argsJson | ConvertFrom-Json)
    if (!($arguments | Where-Object { $_ -match '^/OPT:' })) { throw 'Capture did not specify optimization flags; review defaults before replay' }
    $outputDirectory=Join-Path $fixtureDir 'output'
    New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
    $common=[Collections.Generic.List[string]]::new()
    foreach ($argument in $arguments) {
        if ($argument -match '^/(OUT|PDB|IMPLIB|ILK|INCREMENTAL|REPRODUCE|LINKREPRO):' -or $argument -eq '/INCREMENTAL') { continue }
        if ($argument -ieq '/DEBUG') { $common.Add('/DEBUG:FULL') }
        else { $common.Add($argument) }
    }
    $common.Add('/INCREMENTAL:NO')
    $common.Add('/OUT:' + (Join-Path $outputDirectory 'gitcomet.exe'))
    $common.Add('/PDB:' + (Join-Path $outputDirectory 'gitcomet.pdb'))
    $common.Add('/IMPLIB:' + (Join-Path $outputDirectory 'gitcomet.lib'))
    $common.Add('/ILK:' + (Join-Path $outputDirectory 'gitcomet.ilk'))
    $inputManifest=[Collections.Generic.List[object]]::new()
    foreach ($file in Get-ChildItem -LiteralPath $extract -File -Recurse) {
        $inputManifest.Add(@{path=[IO.Path]::GetRelativePath($extract,$file.FullName);bytes=$file.Length;sha256=(Get-BenchHash $file.FullName)})
    }
    Write-BenchJson (Join-Path $fixtureDir 'input-manifest.json') @($inputManifest)
    $fixture=@{schema_version=1;profile=$Profile;cwd=$cwd;output_directory=$outputDirectory;
        arguments=@($common);fixture_sha256=(Get-BenchHash (Join-Path $fixtureDir 'input-manifest.json'));
        archive_sha256=(Get-BenchHash $archive);input_files=$inputManifest.Count;
        input_bytes=($inputManifest | Measure-Object bytes -Sum).Sum;
        adapter=@{description='Preserve input order/semantics; make full debug explicit; fresh common output; disable incremental linking';original_response=$responsePath}}
    Write-BenchJson (Join-Path $fixtureDir 'fixture.json') $fixture
    Write-Host ('Captured {0}: {1} files, {2:N1} MiB' -f $Profile,$fixture.input_files,($fixture.input_bytes/1MB))
    return $fixture
}
function Get-BenchFixture($Manifest, [string]$Profile) {
    $fixtureDir=Join-Path $Campaign "fixtures/$Profile"
    $fixturePath=Join-Path $fixtureDir 'fixture.json'
    if (Test-Path -LiteralPath $fixturePath) {
        $fixture=Read-BenchJson $fixturePath
        $inputManifestPath=Join-Path $fixtureDir 'input-manifest.json'
        if ((Get-BenchHash $inputManifestPath) -ne $fixture.fixture_sha256) { throw 'Fixture manifest changed' }
        foreach ($inputFile in (Read-BenchJson $inputManifestPath)) {
            $inputPath=Join-Path (Join-Path $fixtureDir 'inputs') $inputFile.path
            if ((Get-BenchHash $inputPath) -ne $inputFile.sha256) { throw "Fixture input changed: $inputPath" }
        }
        return Get-BenchCompactFixture $fixture $Campaign
    }
    if (!(Test-Path -LiteralPath (Join-Path $fixtureDir 'repro.tar'))) {
        Write-Host 'Fetching locked dependencies before the offline preparation build'
        & $Manifest.tools.cargo fetch --locked --manifest-path (Join-Path $Manifest.source 'Cargo.toml') *> (Join-Path $Campaign 'dependency-fetch.log')
        if ($LASTEXITCODE -ne 0) { throw "Dependency fetch failed; see $Campaign/dependency-fetch.log" }
        $target=Join-Path $Campaign "build/capture-$Profile"
        $null=Invoke-BenchCargo $Manifest $Profile lld $target preparation $SampleMs $TimeoutSeconds $fixtureDir
    }
    return Get-BenchCompactFixture (Complete-BenchFixture $Manifest $Profile) $Campaign
}
function Write-BenchResponse([string]$Path, [string[]]$Arguments) {
    # Quote using the same CRT rules as the native collector; each argument on
    # its own line also makes the exact replay command reviewable.
    $lines=foreach ($argument in $Arguments) {
        '"' + [regex]::Replace([regex]::Replace($argument, '(\\*)"', '$1$1\"'), '(\\+)$', '$1$1') + '"'
    }
    [IO.File]::WriteAllText($Path, ($lines -join "`r`n") + "`r`n", [Text.UnicodeEncoding]::new($false,$true))
}
function Save-BenchArtifacts($Manifest, $Fixture, [string]$RunDirectory, [switch]$Validate) {
    $files=@(Get-ChildItem -LiteralPath $Fixture.output_directory -File | Where-Object { $_.Name -match '^gitcomet\.(exe|pdb|lib|exp|ilk)$' } | ForEach-Object {
        @{name=$_.Name;bytes=$_.Length;sha256=(Get-BenchHash $_.FullName)}
    })
    $exe=Join-Path $Fixture.output_directory 'gitcomet.exe'
    if (!(Test-Path -LiteralPath $exe)) { throw 'Link succeeded without producing expected executable' }
    $stream=[IO.File]::OpenRead($exe)
    try {
        $reader=[System.Reflection.PortableExecutable.PEReader]::new($stream)
        $header=$reader.PEHeaders.PEHeader
        $pe=@{machine=$reader.PEHeaders.CoffHeader.Machine.ToString();subsystem=$header.Subsystem.ToString();
            stack_reserve=$header.SizeOfStackReserve;dll_characteristics=$header.DllCharacteristics.ToString();
            sections=@($reader.PEHeaders.SectionHeaders | ForEach-Object { @{name=$_.Name;raw_bytes=$_.SizeOfRawData;virtual_bytes=$_.VirtualSize} });codeview=@()}
        foreach ($debug in $reader.ReadDebugDirectory()) {
            if ($debug.Type.ToString() -eq 'CodeView') {
                $cv=$reader.ReadCodeViewDebugDirectoryData($debug)
                $pe.codeview+=@{guid=$cv.Guid.ToString();age=$cv.Age;path=$cv.Path}
            }
        }
        $reader.Dispose()
    } finally { $stream.Dispose() }
    $artifact=@{files=$files;pe=$pe;smoke=$null;pdb_match=$null}
    if ($Validate) {
        if ($pe.machine -ne 'Amd64' -or $pe.stack_reserve -ne 8388608) { throw 'Unexpected PE target architecture or stack reserve' }
        & $Manifest.tools.dumpbin /HEADERS $exe | Set-Content -LiteralPath (Join-Path $RunDirectory 'pe-headers.txt') -Encoding utf8
        if ($LASTEXITCODE -ne 0) { throw 'PE inspection failed' }
        & $Manifest.tools.dumpbin /DEPENDENTS $exe | Set-Content -LiteralPath (Join-Path $RunDirectory 'pe-dependents.txt') -Encoding utf8
        & $Manifest.tools.dumpbin /PDBPATH:VERBOSE $exe | Set-Content -LiteralPath (Join-Path $RunDirectory 'pdb-validation.txt') -Encoding utf8
        $pdb=Join-Path $Fixture.output_directory 'gitcomet.pdb'
        if ($pe.codeview.Count -gt 0) {
            $identity=Get-BenchPdbIdentity $pdb
            $artifact.pdb_match=($identity.guid -eq $pe.codeview[0].guid -and $identity.age -eq $pe.codeview[0].age)
            if (!$artifact.pdb_match) { throw 'PDB GUID/age does not match the executable CodeView record' }
        }
        $smokeOutput=@(& $exe --version 2>&1)
        $smokeExit=$LASTEXITCODE
        $smokeOutput | Set-Content -LiteralPath (Join-Path $RunDirectory 'smoke-version.log') -Encoding utf8
        if ($smokeExit -ne 0) { throw "Application --version failed: $smokeExit" }
        $artifact.smoke=@{command='--version';exit_code=$smokeExit;output=($smokeOutput -join "`n");gui='not automatically launched';symbols='PE CodeView recorded; dumpbin PDB lookup log retained'}
    }
    Write-BenchJson (Join-Path $RunDirectory 'artifacts.json') $artifact
    return $artifact
}
function Get-BenchPdbIdentity([string]$Path) {
    # MSF 7 superblock -> stream directory -> PDB info stream (stream 1).
    # Read only directory blocks and the info header, even for multi-GiB PDBs.
    $stream=[IO.File]::OpenRead($Path)
    $reader=[IO.BinaryReader]::new($stream)
    try {
        $magic=[Text.Encoding]::ASCII.GetString($reader.ReadBytes(32))
        if (!$magic.StartsWith('Microsoft C/C++ MSF 7.00')) { throw 'Unexpected PDB container format' }
        $blockSize=$reader.ReadUInt32()
        $null=$reader.ReadUInt32(); $null=$reader.ReadUInt32()
        $directoryBytes=$reader.ReadUInt32(); $null=$reader.ReadUInt32(); $blockMap=$reader.ReadUInt32()
        if ($blockSize -lt 512 -or $blockSize -gt 65536 -or $directoryBytes -gt 64MB) { throw 'Invalid PDB directory limits' }
        $directoryBlocks=[int][Math]::Ceiling($directoryBytes/[double]$blockSize)
        $stream.Position=[long]$blockMap*$blockSize
        $blocks=@(for($i=0;$i -lt $directoryBlocks;$i++){$reader.ReadUInt32()})
        $directory=[IO.MemoryStream]::new()
        try {
            foreach($block in $blocks) { $stream.Position=[long]$block*$blockSize; $data=$reader.ReadBytes($blockSize); $directory.Write($data) }
            $directory.Position=0
            $directoryReader=[IO.BinaryReader]::new($directory,[Text.Encoding]::UTF8,$true)
            $streams=$directoryReader.ReadUInt32()
            if ($streams -lt 2 -or $streams -gt $directoryBytes/4) { throw 'Invalid PDB stream count' }
            $stream0Bytes=$directoryReader.ReadUInt32()
            $stream1Bytes=$directoryReader.ReadUInt32()
            if ($stream1Bytes -lt 28 -or $stream1Bytes -eq [uint32]::MaxValue) { throw 'Missing PDB identity stream' }
            $skipBlocks=if($stream0Bytes -eq [uint32]::MaxValue){0}else{[int][Math]::Ceiling($stream0Bytes/[double]$blockSize)}
            $directory.Position=4+4*[long]$streams+4*$skipBlocks
            $infoBlock=$directoryReader.ReadUInt32()
            $stream.Position=[long]$infoBlock*$blockSize
            $null=$reader.ReadUInt32(); $null=$reader.ReadUInt32()
            $age=$reader.ReadUInt32()
            $guid=[guid]::new($reader.ReadBytes(16))
            return @{age=$age;guid=$guid.ToString()}
        } finally { $directory.Dispose() }
    } finally { $reader.Dispose(); $stream.Dispose() }
}
function Invoke-BenchReplay($Manifest, $Fixture, [string]$Backend, [string]$Scenario, [string]$PairId, [int]$Order, [int]$Sampling, [bool]$Warmup=$false, [string]$Diagnostic='none', [bool]$Validate=$false) {
    Set-BenchEnvironment $Manifest $Fixture.profile
    # All libraries must resolve from the recorded response/fixture, not an
    # accidentally inherited SDK directory.
    $env:LIB=''
    $env:LIBPATH=''
    [Environment]::SetEnvironmentVariable('GITCOMET_LINK_BENCH_CONFIG',$null,'Process')
    Reset-BenchDirectory $Fixture.output_directory $Campaign
    $runId=New-BenchId "$($Fixture.profile)-$Scenario-$Backend"
    $runDirectory=Join-Path $Campaign "runs/$runId"
    New-Item -ItemType Directory -Path $runDirectory -Force | Out-Null
    $metadata=New-BenchMetadata $Manifest $Fixture.profile $Scenario $Backend 'linker' $runId
    $metadata.fixture_sha256=$Fixture.fixture_sha256
    $metadata.pair_id=$PairId; $metadata.order=$Order; $metadata.warmup=$Warmup; $metadata.diagnostic_mode=$Diagnostic
    $metadata.application_link=$true; $metadata.lld_threads=$LldThreads
    $nativeArgs=@($Fixture.arguments)
    if ($Diagnostic -ne 'none') { $nativeArgs+='/TIME' }
    if ($Backend -eq 'lld' -and $Diagnostic -eq 'phase-trace') { $nativeArgs+='--time-trace=' + (Join-Path $runDirectory 'lld-time-trace.json') }
    if ($Backend -eq 'lld' -and $LldThreads -gt 0) { $nativeArgs+='/threads:' + $LldThreads }
    $response=Join-Path $runDirectory 'link.rsp'
    Write-BenchResponse $response $nativeArgs
    $metadata.response_sha256=Get-BenchHash $response
    $arguments=@('@' + $response)
    if ($Backend -eq 'lld') { $arguments=@('-flavor','link')+$arguments }
    Write-Host "[$([DateTime]::Now.ToString('HH:mm:ss'))] $($Fixture.profile) $Scenario $Backend pair=$PairId sample=${Sampling}ms"
    $metrics=Invoke-BenchProcess $Manifest $Manifest.tools[$Backend] $arguments $Fixture.cwd $runDirectory $metadata $Sampling $TimeoutSeconds
    try {
        $diagnostics=(Get-Content -LiteralPath (Join-Path $runDirectory 'stdout.log'),(Join-Path $runDirectory 'stderr.log') -Raw) -join "`n"
        if ($diagnostics -match '(?i)LNK4044|unknown argument|unknown option|ignoring unknown|LNK4075') { throw "Incompatible/ignored linker options: $runDirectory" }
        $artifact=Save-BenchArtifacts $Manifest $Fixture $runDirectory -Validate:$Validate
    } catch {
        $metrics.valid=$false
        $metrics.validation_error=$_.Exception.Message
        Write-BenchJson (Join-Path $runDirectory 'metrics.json') $metrics
        throw
    }
    $metrics.artifacts=$artifact
    $metrics.warning_count=([regex]::Matches($diagnostics,'(?im)\bwarning\b')).Count
    Write-BenchJson (Join-Path $runDirectory 'metrics.json') $metrics
    Write-Host ('  {0:N3}s; CPU {1:N3}s; peak commit {2:N1} MiB' -f $metrics.wall_s,$metrics.tree_cpu_s,($metrics.peak_tree_commit_bytes/1MB))
    return $metrics
}
function Invoke-BenchSeries($Manifest, $Fixture, [string]$Scenario='replay') {
    # Preflight and warmups are retained but excluded from speedup summaries.
    $preflight=@{}
    foreach ($backend in @('msvc','lld')) { $preflight[$backend]=Invoke-BenchReplay $Manifest $Fixture $backend preflight '' 0 $SampleMs $true none $true }
    foreach ($property in @('machine','subsystem','stack_reserve','dll_characteristics')) {
        if ($preflight.msvc.artifacts.pe[$property] -ne $preflight.lld.artifacts.pe[$property]) { throw "Linkers produced different PE $property values" }
    }
    $dependencies=@{}
    foreach ($backend in @('msvc','lld')) {
        $text=Get-Content -LiteralPath (Join-Path $preflight[$backend].output_dir 'pe-dependents.txt') -Raw
        $dependencies[$backend]=(@([regex]::Matches($text,'(?im)^\s*([\w.\-]+\.dll)\s*$') | ForEach-Object { $_.Groups[1].Value.ToLowerInvariant() }) | Sort-Object -Unique) -join ','
    }
    if ($dependencies.msvc -ne $dependencies.lld) { throw 'Linkers produced different DLL dependency lists' }
    for ($i=0; $i -lt $Warmups; $i++) {
        foreach ($backend in @('lld','msvc')) { $null=Invoke-BenchReplay $Manifest $Fixture $backend warmup '' 0 $SampleMs $true }
    }
    $seriesId=New-BenchId $Scenario
    $orders=@(Get-BenchPairOrder $Pairs $Manifest.seed)
    Write-BenchJson (Join-Path $Campaign "$seriesId-order.json") @{seed=$Manifest.seed;orders=$orders;profile=$Fixture.profile;sample_ms=$SampleMs;lld_threads=$LldThreads}
    for ($pair=0; $pair -lt $Pairs; $pair++) {
        for ($order=0; $order -lt 2; $order++) {
            $null=Invoke-BenchReplay $Manifest $Fixture $orders[$pair][$order] $Scenario "$seriesId-$pair" $order $SampleMs
        }
    }
}
function Invoke-BenchCalibration($Manifest, $Fixture) {
    $seriesId=New-BenchId 'calibration'
    foreach ($backend in @('msvc','lld')) {
        $null=Invoke-BenchReplay $Manifest $Fixture $backend calibration-warmup '' 0 $SampleMs $true
        for ($pair=0; $pair -lt 3; $pair++) {
            $intervals=if ($pair % 2) { @($SampleMs,0) } else { @(0,$SampleMs) }
            for ($order=0; $order -lt 2; $order++) {
                $null=Invoke-BenchReplay $Manifest $Fixture $backend calibration "$seriesId-$backend-$pair" $order $intervals[$order]
            }
        }
    }
}
function Invoke-BenchDiagnostics($Manifest, $Fixture) {
    foreach ($backend in @('msvc','lld')) {
        $null=Invoke-BenchReplay $Manifest $Fixture $backend diagnostics '' 0 $SampleMs $false phase-trace
    }
    if ($Etw) {
        $diagnosticDir=Join-Path $Campaign ('diagnostics/' + (New-BenchId $Fixture.profile))
        New-Item -ItemType Directory -Path $diagnosticDir -Force | Out-Null
        & wpr -profiledetails GeneralProfile | Set-Content -LiteralPath (Join-Path $diagnosticDir 'wpr-profile.txt') -Encoding utf8
        foreach ($backend in @('msvc','lld')) {
            $status=(& wpr -status 2>&1) -join "`n"
            if ($status -notmatch 'WPR is not recording') { throw "WPR is already recording or its status is unknown; existing recording left intact: $status" }
            & wpr -start GeneralProfile -filemode
            if ($LASTEXITCODE -ne 0) { throw 'WPR could not start; kernel tracing may require an elevated shell' }
            $etl=Join-Path $diagnosticDir "$backend.etl"
            try { $null=Invoke-BenchReplay $Manifest $Fixture $backend diagnostics-etw '' 0 $SampleMs $false etw }
            finally {
                & wpr -stop $etl
                if ($LASTEXITCODE -ne 0) { Write-Warning 'WPR stop failed; inspect the recording manually' }
            }
            if ($WpaProfile) {
                $exporter=Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10/Windows Performance Toolkit/wpaexporter.exe'
                $export=Join-Path $diagnosticDir "$backend-tables"
                New-Item -ItemType Directory -Path $export -Force | Out-Null
                & $exporter -i $etl -profile $WpaProfile -outputfolder $export
                if ($LASTEXITCODE -ne 0) { throw 'WPA export failed' }
                Copy-Item -LiteralPath $WpaProfile -Destination (Join-Path $diagnosticDir 'export.wpaProfile') -Force
            }
        }
    }
}
function Invoke-BenchBuildSeries($Manifest, [string]$Profile) {
    $seriesId=New-BenchId "build-$Profile"
    $orders=@(Get-BenchPairOrder $BuildPairs ($Manifest.seed+1))
    $main=Join-Path $Manifest.source 'crates/gitcomet/src/main.rs'
    $original=[IO.File]::ReadAllBytes($main)
    $originalTime=[IO.File]::GetLastWriteTimeUtc($main)
    # A referenced executable string changes code, while keeping behavior of
    # ordinary invocations unchanged. This patch lives only in the snapshot.
    $sourceText=[Text.Encoding]::UTF8.GetString($original)
    $mainPattern='(?m)^fn main\(\)\s*\{'
    if ($sourceText -notmatch $mainPattern) { throw 'Cannot locate main() for controlled edit fixture' }
    $baseText=[regex]::Replace($sourceText,$mainPattern,'fn main() { if std::env::var_os("GITCOMET_BENCH_EDIT_PROBE").is_some() { eprintln!("linker-bench-before"); }',1)
    $editText=$baseText.Replace('linker-bench-before','linker-bench-after!')
    Write-BenchJson (Join-Path $Campaign "$seriesId-patch.json") @{path='crates/gitcomet/src/main.rs';before=$baseText;after=$editText;description='Change a runtime-referenced string in main; environment branch is inactive during normal use'}
    try {
        foreach ($scenario in $BuildScenarios) {
            $target=Join-Path $Campaign "build/$Profile-$scenario-active"
            if ($scenario -in @('edit','noop')) {
                foreach ($backend in @('msvc','lld')) {
                    Reset-BenchDirectory $target $Campaign
                    if ($scenario -eq 'edit') { [IO.File]::WriteAllText($main,$baseText,[Text.UTF8Encoding]::new($false)) }
                    else { [IO.File]::WriteAllBytes($main,$original) }
                    $null=Invoke-BenchCargo $Manifest $Profile $backend $target build-preparation $SampleMs $TimeoutSeconds
                    $snapshot=Join-Path $Campaign "build/$Profile-$scenario-$backend-snapshot"
                    Reset-BenchDirectory $snapshot $Campaign
                    Copy-BenchTree $target $snapshot $Campaign
                }
            }
            for ($pair=0; $pair -lt $BuildPairs; $pair++) {
                for ($order=0; $order -lt 2; $order++) {
                    $backend=$orders[$pair][$order]
                    Reset-BenchDirectory $target $Campaign
                    if ($scenario -in @('edit','noop')) {
                        Copy-BenchTree (Join-Path $Campaign "build/$Profile-$scenario-$backend-snapshot") $target $Campaign
                    }
                    if ($scenario -eq 'edit') { [IO.File]::WriteAllText($main,$editText,[Text.UTF8Encoding]::new($false)) }
                    else {
                        [IO.File]::WriteAllBytes($main,$original)
                        [IO.File]::SetLastWriteTimeUtc($main,$originalTime)
                    }
                    $metrics=Invoke-BenchCargo $Manifest $Profile $backend $target "build-$scenario" $SampleMs $TimeoutSeconds '' @{pair_id="$seriesId-$scenario-$pair";order=$order;output_state=$(if($scenario -eq 'clean'){'empty-target'}else{'restored-cache'})}
                    if ($scenario -eq 'noop' -and $metrics.link_invocations -ne 0) { throw 'No-op unexpectedly invoked a linker; inspect build fingerprints' }
                    if ($scenario -eq 'edit' -and $metrics.link_invocations -eq 0) { throw 'Controlled edit did not invoke a linker' }
                }
            }
            # Large debug caches are temporary; metrics, timings, and artifact
            # metadata were copied into runs before this scoped cleanup.
            foreach ($cache in @($target,(Join-Path $Campaign "build/$Profile-$scenario-msvc-snapshot"),(Join-Path $Campaign "build/$Profile-$scenario-lld-snapshot"))) {
                $checked=Assert-BenchDescendant $cache $Campaign
                if (Test-Path -LiteralPath $checked) { Remove-Item -LiteralPath $checked -Recurse -Force }
            }
        }
    } finally { [IO.File]::WriteAllBytes($main,$original); [IO.File]::SetLastWriteTimeUtc($main,$originalTime) }
}

try {
    $manifestPath=Join-Path $Campaign 'manifest.json'
    $manifest=if (Test-Path -LiteralPath $manifestPath) { Read-BenchJson $manifestPath } else { New-BenchCampaign $repo $Campaign $Jobs $Seed }
    Assert-BenchTools $manifest
    # Do not run two competing measurement stages in the same campaign.
    $lockPath=Join-Path $Campaign 'campaign.lock'
    $lock=[IO.File]::Open($lockPath,[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
    Write-Host "Campaign: $Campaign"
    try {
        Write-BenchJson (Join-Path $Campaign 'status.json') @{state='running';stage=$Stage;pid=$PID;profiles=$Profiles;updated_utc=[DateTime]::UtcNow.ToString('o')}
        foreach ($profile in $Profiles) {
            if ($Stage -eq 'Initialize') { break }
            if ($Stage -eq 'Build') { Invoke-BenchBuildSeries $manifest $profile; continue }
            $fixture=Get-BenchFixture $manifest $profile
            if ($Stage -in @('Calibrate','All')) { Invoke-BenchCalibration $manifest $fixture }
            if ($Stage -in @('Replay','All')) { Invoke-BenchSeries $manifest $fixture }
            if ($Stage -in @('Diagnostics','All')) { Invoke-BenchDiagnostics $manifest $fixture }
            if ($Stage -eq 'All') { Invoke-BenchBuildSeries $manifest $profile }
        }
        Write-BenchJson (Join-Path $Campaign 'status.json') @{state='completed';stage=$Stage;profiles=$Profiles;updated_utc=[DateTime]::UtcNow.ToString('o')}
    } catch {
        Write-BenchJson (Join-Path $Campaign 'status.json') @{state=$(if($_.Exception -is [OperationCanceledException]){'stopped'}else{'failed'});stage=$Stage;error=$_.Exception.Message;updated_utc=[DateTime]::UtcNow.ToString('o')}
        throw
    } finally { $lock.Dispose() }
    $summary=Join-Path $PSScriptRoot 'summarize-linkers.ps1'
    if (Test-Path -LiteralPath $summary) { & $summary -Campaign $Campaign }
} finally {
    foreach ($item in @(Get-ChildItem Env:)) { if (!$savedEnvironment.ContainsKey($item.Name)) { [Environment]::SetEnvironmentVariable($item.Name,$null,'Process') } }
    foreach ($key in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($key,$savedEnvironment[$key],'Process') }
}
