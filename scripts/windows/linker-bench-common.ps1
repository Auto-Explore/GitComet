Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Write-BenchJson($Path, $Value) {
    $Value | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath $Path -Encoding utf8
}
function Read-BenchJson($Path) { Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json -AsHashtable }
function Get-BenchHash($Path) { (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() }
function Get-TextHash([string]$Text) {
    [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($Text))).ToLowerInvariant()
}
function New-BenchId([string]$Prefix) { '{0}-{1}-{2}' -f $Prefix, [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssfff'), [guid]::NewGuid().ToString('N').Substring(0,8) }
function Assert-BenchDescendant([string]$Path, [string]$Root) {
    $resolvedRoot = [IO.Path]::GetFullPath($Root).TrimEnd('\','/')
    $resolvedPath = [IO.Path]::GetFullPath($Path).TrimEnd('\','/')
    if (!$resolvedPath.StartsWith($resolvedRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing operation outside benchmark directory: $resolvedPath"
    }
    $cursor = $resolvedPath
    while ($cursor.Length -gt $resolvedRoot.Length) {
        if ((Test-Path -LiteralPath $cursor) -and ((Get-Item -LiteralPath $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Refusing operation through reparse point: $cursor"
        }
        $cursor = [IO.Path]::GetDirectoryName($cursor)
    }
    return $resolvedPath
}
function Reset-BenchDirectory([string]$Path, [string]$Root) {
    $checkedPath = Assert-BenchDescendant $Path $Root
    if (Test-Path -LiteralPath $checkedPath) { Remove-Item -LiteralPath $checkedPath -Recurse -Force }
    New-Item -ItemType Directory -Path $checkedPath -Force | Out-Null
}
function Copy-BenchTree([string]$Source, [string]$Destination, [string]$Root) {
    $checkedDestination = Assert-BenchDescendant $Destination $Root
    New-Item -ItemType Directory -Path $checkedDestination -Force | Out-Null
    & robocopy $Source $checkedDestination /E /COPY:DAT /DCOPY:T /R:1 /W:1 /XJ /NFL /NDL /NJH /NJS /NP | Out-Null
    if ($LASTEXITCODE -ge 8) { throw "robocopy failed: $LASTEXITCODE" }
}
function Get-BenchCompactFixture($Fixture, [string]$Campaign) {
    # Reproduction paths repeat the original tree and can exceed LINK's path
    # limit. Both backends receive identical copies in the same short layout.
    $root=Join-Path $Campaign "fixtures/$($Fixture.profile)/short"
    $null=Assert-BenchDescendant $root $Campaign
    New-Item -ItemType Directory -Path $root -Force | Out-Null
    $directories=@{}
    function Convert-InputDirectory([string]$Directory) {
        $full=[IO.Path]::GetFullPath($Directory)
        $null=Assert-BenchDescendant (Join-Path $full '__path_check__') $Fixture.cwd
        if (!$directories.ContainsKey($full)) {
            $relative=[IO.Path]::GetRelativePath($Fixture.cwd,$full)
            $short=Join-Path $root (Get-TextHash $relative).Substring(0,16)
            $null=Assert-BenchDescendant $short $Campaign
            New-Item -ItemType Directory -Path $short -Force | Out-Null
            $directories[$full]=$short
        }
        return $directories[$full]
    }
    $files=@(foreach ($file in Get-ChildItem -LiteralPath $Fixture.cwd -File -Recurse | Sort-Object FullName) {
        $destination=Join-Path (Convert-InputDirectory $file.DirectoryName) $file.Name
        if ($destination.Length -ge 260) { throw "Campaign path is too long for MSVC replay: $destination" }
        $hash=Get-BenchHash $file.FullName
        if (!(Test-Path -LiteralPath $destination)) { Copy-Item -LiteralPath $file.FullName -Destination $destination }
        if ((Get-BenchHash $destination) -ne $hash) { throw "Compact fixture input changed: $destination" }
        @{original=[IO.Path]::GetRelativePath($Fixture.cwd,$file.FullName);path=$destination;sha256=$hash}
    })
    $arguments=@(foreach ($argument in $Fixture.arguments) {
        if ($argument -match '^/libpath:(.*)$') {
            '/libpath:' + (Convert-InputDirectory (Join-Path $Fixture.cwd $Matches[1]))
        } elseif ($argument -match '^(?<prefix>/natvis:|/def:)?(?<path>[^/].*)$') {
            $prefix=if($Matches.ContainsKey('prefix')){$Matches.prefix}else{''}
            $path=$Matches.path
            $original=Join-Path $Fixture.cwd $path
            if (Test-Path -LiteralPath $original -PathType Leaf) {
                $prefix + (Join-Path (Convert-InputDirectory ([IO.Path]::GetDirectoryName($original))) ([IO.Path]::GetFileName($original)))
            } else { $argument }
        } else { $argument }
    })
    $layout=@{adapter='compact-paths-v1';original_fixture_sha256=$Fixture.fixture_sha256;arguments=$arguments;files=$files}
    $layoutPath=Join-Path $root 'layout.json'
    $identity=Get-TextHash (($arguments -join "`n") + "`n" + $Fixture.fixture_sha256 + "`ncompact-paths-v1")
    Write-BenchJson $layoutPath $layout
    $compact=$Fixture.Clone()
    $compact.arguments=$arguments
    $compact.cwd=$root
    $compact.fixture_sha256=$identity
    $compact.adapter=@{description='Byte-identical compact input paths for both linkers';layout=$layoutPath;original_fixture_sha256=$Fixture.fixture_sha256}
    return $compact
}
function Get-BenchEnvironment {
    $result = [ordered]@{}
    Get-ChildItem Env: | Where-Object { $_.Name -match '^(PATH|LIB|LIBPATH|INCLUDE|LINK|_LINK_|RUSTC|RUSTFLAGS|CARGO_ENCODED_RUSTFLAGS|RUSTC_WRAPPER|RUSTC_WORKSPACE_WRAPPER|CARGO_HOME|RUSTUP_HOME|CARGO_BUILD_JOBS|CARGO_BUILD_TARGET|CARGO_TARGET_DIR|CARGO_INCREMENTAL|CARGO_PROFILE_.*|CARGO_TARGET_.*|GITCOMET_LINK_STACK_RESERVE)$' } |
        Sort-Object Name | ForEach-Object { $result[$_.Name] = $_.Value }
    return $result
}
function Set-BenchEnvironment($Manifest, [string]$Profile) {
    $tool = $Manifest.tools
    $env:LIB = $tool.library_paths -join ';'
    $env:LIBPATH = $env:LIB
    $env:INCLUDE = $tool.include_paths -join ';'
    $env:PATH = $tool.execution_path
    foreach ($name in @('LINK','_LINK_','RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','RUSTC_WRAPPER','RUSTC_WORKSPACE_WRAPPER','CARGO_INCREMENTAL','CARGO_TARGET_DIR','CARGO_BUILD_TARGET')) {
        [Environment]::SetEnvironmentVariable($name, $null, 'Process')
    }
    foreach ($item in @(Get-ChildItem Env: | Where-Object { $_.Name -match '^CARGO_PROFILE_' })) {
        [Environment]::SetEnvironmentVariable($item.Name, $null, 'Process')
    }
    $env:RUSTC = $tool.rustc
    $env:CARGO_BUILD_JOBS = [string]$Manifest.cargo_jobs
    $env:GITCOMET_LINK_STACK_RESERVE = '8388608'
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = $Manifest.helper
    if ($Profile -eq 'shipping-release') { $env:CARGO_PROFILE_RELEASE_DEBUG = 'line-tables-only' }
}
function Get-BenchToolchain {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (!$vs) { throw 'No Visual Studio x64 C++ toolset found' }
    $msvc = Get-ChildItem -LiteralPath (Join-Path $vs 'VC/Tools/MSVC') -Directory | Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
    $sdkRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10'
    $sdk = Get-ChildItem -LiteralPath (Join-Path $sdkRoot 'Lib') -Directory | Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'um/x64/kernel32.lib') } | Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
    $rustc = & rustup which rustc
    if ($LASTEXITCODE -ne 0) { throw 'rustup which rustc failed' }
    $cargo = & rustup which cargo
    $rustLib = & $rustc --print target-libdir
    $lld = [IO.Path]::GetFullPath((Join-Path $rustLib '../bin/rust-lld.exe'))
    $bin = Join-Path $msvc.FullName 'bin/Hostx64/x64'
    $link = Join-Path $bin 'link.exe'
    foreach ($file in @($lld,$link,$rustc,$cargo)) { if (!(Test-Path -LiteralPath $file)) { throw "Missing tool: $file" } }
    return [ordered]@{
        rustc=$rustc; cargo=$cargo; rustc_version=((& $rustc -vV) -join "`n"); cargo_version=(& $cargo -V)
        lld=$lld; lld_version=((& $lld -flavor link --version) -join "`n"); lld_sha256=(Get-BenchHash $lld)
        msvc=$link; msvc_version=(Get-Item -LiteralPath $link).VersionInfo.FileVersion; msvc_sha256=(Get-BenchHash $link)
        rustc_sha256=(Get-BenchHash $rustc); msvc_toolset=$msvc.Name; sdk_version=$sdk.Name
        dumpbin=(Join-Path $bin 'dumpbin.exe')
        library_paths=@((Join-Path $msvc.FullName 'lib/x64'),(Join-Path $sdk.FullName 'um/x64'),(Join-Path $sdk.FullName 'ucrt/x64'))
        include_paths=@((Join-Path $msvc.FullName 'include'),(Join-Path $sdkRoot "Include/$($sdk.Name)/ucrt"),(Join-Path $sdkRoot "Include/$($sdk.Name)/um"),(Join-Path $sdkRoot "Include/$($sdk.Name)/shared"))
        execution_path=(@($bin,(Join-Path $sdkRoot "bin/$($sdk.Name)/x64"),$env:PATH) -join ';')
    }
}
function New-BenchCampaign([string]$Repo, [string]$Campaign, [int]$Jobs, [int]$Seed) {
    if (Test-Path -LiteralPath (Join-Path $Campaign 'manifest.json')) { throw 'Campaign already initialized' }
    New-Item -ItemType Directory -Path $Campaign -Force | Out-Null
    $tool = Get-BenchToolchain
    $helperTarget = Join-Path $Repo 'target/linker-bench-tools'
    & $tool.cargo build --release --locked --offline --manifest-path (Join-Path $Repo 'scripts/windows/linker-metrics/Cargo.toml') --target-dir $helperTarget
    if ($LASTEXITCODE -ne 0) { throw 'Building metrics helper failed' }
    # Freeze the helper per campaign: subsequent tool edits cannot change a run.
    $helper = Join-Path $Campaign 'gitcomet-linker-metrics.exe'
    Copy-Item -LiteralPath (Join-Path $helperTarget 'release/gitcomet-linker-metrics.exe') -Destination $helper
    $source = Join-Path $Campaign 'source'
    New-Item -ItemType Directory -Path $source -Force | Out-Null
    $paths = ((& git -C $Repo ls-files -z --cached --others --exclude-standard) -join "`n").Split([char]0, [StringSplitOptions]::RemoveEmptyEntries)
    if ($LASTEXITCODE -ne 0) { throw 'Could not enumerate source snapshot' }
    $sourceManifest = [Collections.Generic.List[object]]::new()
    foreach ($relative in $paths) {
        $from = Join-Path $Repo $relative
        if (!(Test-Path -LiteralPath $from -PathType Leaf)) { continue }
        $to = Assert-BenchDescendant (Join-Path $source $relative) $source
        [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($to)) | Out-Null
        [IO.File]::Copy($from, $to, $false)
        $sourceManifest.Add(@{path=$relative; bytes=(Get-Item -LiteralPath $to).Length; sha256=(Get-BenchHash $to)})
    }
    Write-BenchJson (Join-Path $Campaign 'source-manifest.json') @($sourceManifest)
    & git -C $Repo diff --binary HEAD | Set-Content -LiteralPath (Join-Path $Campaign 'working-tree.patch') -Encoding utf8
    $machine = [ordered]@{
        cpu=@(Get-CimInstance Win32_Processor | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors)
        memory_bytes=(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory
        os=(Get-CimInstance Win32_OperatingSystem | Select-Object Caption,Version,BuildNumber)
        disks=@(Get-Disk | Select-Object Number,FriendlyName,BusType,Size)
        volumes=@(Get-Volume | Where-Object DriveLetter | Select-Object DriveLetter,FileSystem,Size,SizeRemaining)
        partitions=@(Get-Partition | Where-Object DriveLetter | Select-Object DriveLetter,DiskNumber,PartitionNumber)
        power_scheme=((& powercfg /getactivescheme) -join "`n")
        defender=$null
    }
    try { $machine.defender = Get-MpComputerStatus | Select-Object AntivirusEnabled,RealTimeProtectionEnabled,AntivirusSignatureVersion } catch { $machine.defender = @{unavailable=$_.Exception.Message} }
    $manifest = [ordered]@{
        schema_version=1; created_utc=[DateTime]::UtcNow.ToString('o'); campaign_id=(Split-Path $Campaign -Leaf)
        campaign=$Campaign; source=$source; source_sha256=(Get-BenchHash (Join-Path $Campaign 'source-manifest.json'))
        git_commit=(& git -C $Repo rev-parse HEAD); git_status=@(& git -C $Repo status --short)
        tools=$tool; helper=$helper; helper_sha256=(Get-BenchHash $helper); machine=$machine
        original_environment=(Get-BenchEnvironment); cargo_jobs=$Jobs; seed=$Seed
        policy=@{cache='warm-filesystem'; incremental_linking=$false; output_state='fresh'; compiler_cache='disabled'; stack_bytes=8388608; target='x86_64-pc-windows-msvc'; target_cpu='x86-64-v3'}
    }
    Write-BenchJson (Join-Path $Campaign 'manifest.json') $manifest
    return $manifest
}
function Assert-BenchTools($Manifest) {
    foreach ($name in @('lld','msvc','rustc')) {
        if ((Get-BenchHash $Manifest.tools[$name]) -ne $Manifest.tools["${name}_sha256"]) { throw "Tool changed during campaign: $name" }
    }
    if ((Get-BenchHash $Manifest.helper) -ne $Manifest.helper_sha256) { throw 'Campaign helper changed' }
}
function Invoke-BenchProcess($Manifest, [string]$Executable, [string[]]$Arguments, [string]$Cwd, [string]$RunDirectory, $Metadata, [int]$SampleMs=50, [int]$TimeoutSeconds=7200) {
    if (Test-Path -LiteralPath (Join-Path $Manifest.campaign 'stop.request')) { throw [OperationCanceledException]::new('Stop requested; no new process started') }
    New-Item -ItemType Directory -Path $RunDirectory -Force | Out-Null
    Write-BenchJson (Join-Path $Manifest.campaign 'current-run.json') @{metadata=$Metadata;directory=$RunDirectory;updated_utc=[DateTime]::UtcNow.ToString('o')}
    $specPath = Join-Path $RunDirectory 'spec.json'
    Write-BenchJson $specPath @{executable=$Executable; arguments=@($Arguments); cwd=$Cwd; output_dir=$RunDirectory; sample_ms=$SampleMs; timeout_s=$TimeoutSeconds; metadata=$Metadata}
    & $Manifest.helper --run-spec $specPath
    $exit = $LASTEXITCODE
    $metricsPath = Join-Path $RunDirectory 'metrics.json'
    if (!(Test-Path -LiteralPath $metricsPath)) { throw "Collector failed without a result ($exit): $specPath" }
    $metrics = Read-BenchJson $metricsPath
    if ($exit -ne 0 -or !$metrics.valid) {
        foreach ($log in @('stdout.log','stderr.log')) {
            if (Test-Path -LiteralPath (Join-Path $RunDirectory $log)) { Get-Content -LiteralPath (Join-Path $RunDirectory $log) -Tail 50 | ForEach-Object { Write-Host $_ } }
        }
        throw "Measured process failed ($exit): $metricsPath"
    }
    return $metrics
}
function New-BenchMetadata($Manifest, [string]$Profile, [string]$Scenario, [string]$Backend, [string]$Scope, [string]$RunId) {
    return @{campaign_id=$Manifest.campaign_id; source_sha256=$Manifest.source_sha256; profile=$Profile; scenario=$Scenario;
        linker=$Backend; scope=$Scope; run_id=$RunId; cache_policy='warm-filesystem'; output_state='fresh'; diagnostic_mode='none';
        pair_id=$null; order=$null; fixture_sha256=$null; parent_build_id=$null; warmup=$false;
        collector_sha256=$Manifest.helper_sha256; backend_sha256=$Manifest.tools["${Backend}_sha256"]}
}
function Invoke-BenchCargo($Manifest, [string]$Profile, [string]$Backend, [string]$TargetDirectory, [string]$Scenario, [int]$SampleMs, [int]$TimeoutSeconds, [string]$CaptureDirectory='', $ExtraMetadata=@{}) {
    Set-BenchEnvironment $Manifest $Profile
    $runId = New-BenchId "$Profile-$Scenario-$Backend"
    $runDirectory = Join-Path $Manifest.campaign "runs/$runId"
    New-Item -ItemType Directory -Path $runDirectory -Force | Out-Null
    $metadata = New-BenchMetadata $Manifest $Profile $Scenario $Backend 'cargo' $runId
    foreach ($key in $ExtraMetadata.Keys) { $metadata[$key]=$ExtraMetadata[$key] }
    $proxyMetadata = $metadata.Clone()
    $proxyMetadata.parent_build_id=$runId
    $config = @{backend=$Backend; executable=$Manifest.tools[$Backend]; log_root=(Join-Path $runDirectory 'links'); sample_ms=$SampleMs; metadata=$proxyMetadata}
    if ($CaptureDirectory) { $config.capture_dir=$CaptureDirectory }
    $configPath = Join-Path $runDirectory 'proxy.json'
    Write-BenchJson $configPath $config
    $env:GITCOMET_LINK_BENCH_CONFIG=$configPath
    $arguments = @('build','--locked','--offline','-p','gitcomet','--bin','gitcomet','--target','x86_64-pc-windows-msvc','--timings','--target-dir',$TargetDirectory)
    if ($Profile -eq 'shipping-release' -or $Profile -eq 'release') { $arguments += '--release' }
    elseif ($Profile -eq 'release-with-debug') { $arguments += @('--profile','release-with-debug') }
    Write-Host "[$([DateTime]::Now.ToString('HH:mm:ss'))] $Profile $Scenario $Backend; logs: $runDirectory"
    $metrics = Invoke-BenchProcess $Manifest $Manifest.tools.cargo $arguments $Manifest.source $runDirectory $metadata $SampleMs $TimeoutSeconds
    $timings = Join-Path $TargetDirectory 'cargo-timings'
    if (Test-Path -LiteralPath $timings) { Copy-BenchTree $timings (Join-Path $runDirectory 'cargo-timings') $Manifest.campaign }
    $linkResults = @(Get-ChildItem -LiteralPath (Join-Path $runDirectory 'links') -Filter metrics.json -Recurse -ErrorAction SilentlyContinue)
    $metrics.link_invocations=$linkResults.Count
    $profileDirectory=if($Profile -eq 'dev'){'debug'}elseif($Profile -eq 'shipping-release'){'release'}else{$Profile}
    $artifactDirectory=Join-Path $TargetDirectory "x86_64-pc-windows-msvc/$profileDirectory"
    if (Test-Path -LiteralPath (Join-Path $artifactDirectory 'gitcomet.exe')) {
        $metrics.artifacts=Save-BenchArtifacts $Manifest @{output_directory=$artifactDirectory} $runDirectory
    }
    Write-BenchJson (Join-Path $runDirectory 'metrics.json') $metrics
    Write-Host ('Completed {0}: {1:N3}s; intercepted links: {2}' -f $runId,$metrics.wall_s,$linkResults.Count)
    return $metrics
}
function Get-BenchPairOrder([int]$Pairs, [int]$Seed) {
    $random = [Random]::new($Seed)
    $orders = [Collections.Generic.List[int]]::new()
    for ($i=0; $i -lt $Pairs; $i++) { $orders.Add($i % 2) }
    for ($i=$orders.Count-1; $i -gt 0; $i--) { $j=$random.Next($i+1); $tmp=$orders[$i]; $orders[$i]=$orders[$j]; $orders[$j]=$tmp }
    foreach ($order in $orders) { Write-Output -NoEnumerate $(if ($order -eq 0) { @('msvc','lld') } else { @('lld','msvc') }) }
}
