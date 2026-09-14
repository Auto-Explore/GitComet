#requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Campaign,
    [ValidateRange(100,100000)][int]$BootstrapSamples=2000
)
. (Join-Path $PSScriptRoot 'linker-bench-common.ps1')
$Campaign=[IO.Path]::GetFullPath($Campaign)
$manifest=Read-BenchJson (Join-Path $Campaign 'manifest.json')

function Get-Quantile([double[]]$Values, [double]$Probability) {
    if ($Values.Count -eq 0) { return $null }
    $sorted=@($Values | Sort-Object)
    $position=($sorted.Count-1)*$Probability
    $lower=[int][Math]::Floor($position)
    $upper=[int][Math]::Ceiling($position)
    return $sorted[$lower]+($sorted[$upper]-$sorted[$lower])*($position-$lower)
}
function Get-Statistics([object[]]$Values) {
    [double[]]$available=@($Values | Where-Object { $null -ne $_ })
    if ($available.Count -eq 0) { return @{n=0;median=$null;mean=$null;sd=$null;min=$null;max=$null;q25=$null;q75=$null;p95=$null} }
    $mean=($available | Measure-Object -Average).Average
    $variance=0.0
    foreach ($value in $available) { $variance+=($value-$mean)*($value-$mean) }
    return @{n=$available.Count;median=(Get-Quantile $available 0.5);mean=$mean;
        sd=$(if($available.Count -gt 1){[Math]::Sqrt($variance/($available.Count-1))}else{$null});
        min=($available | Measure-Object -Minimum).Minimum;max=($available | Measure-Object -Maximum).Maximum;
        q25=(Get-Quantile $available 0.25);q75=(Get-Quantile $available 0.75);
        p95=$(if($available.Count -ge 30){Get-Quantile $available 0.95}else{$null})}
}
function Get-BootstrapInterval([double[]]$Values, [int]$Seed) {
    if ($Values.Count -lt 2) { return @($null,$null) }
    $random=[Random]::new($Seed)
    $replicates=[double[]]::new($BootstrapSamples)
    $sample=[double[]]::new($Values.Count)
    for ($i=0; $i -lt $BootstrapSamples; $i++) {
        for ($j=0; $j -lt $Values.Count; $j++) { $sample[$j]=$Values[$random.Next($Values.Count)] }
        [Array]::Sort($sample)
        $middle=[int][Math]::Floor($sample.Length/2)
        $replicates[$i]=if($sample.Length % 2){$sample[$middle]}else{($sample[$middle-1]+$sample[$middle])/2}
    }
    return @((Get-Quantile $replicates 0.025),(Get-Quantile $replicates 0.975))
}
function Get-ArtifactBytes($Metric,[string]$Extension) {
    if (!$Metric.ContainsKey('artifacts')) { return $null }
    $files=@($Metric.artifacts.files | Where-Object { [IO.Path]::GetExtension($_.name) -eq $Extension })
    if (!$files.Count) { return $null }
    return ($files | Measure-Object bytes -Sum).Sum
}

$metrics=[Collections.Generic.List[object]]::new()
$rows=[Collections.Generic.List[object]]::new()
$runRoot=Join-Path $Campaign 'runs'
if (!(Test-Path -LiteralPath $runRoot)) { Write-Host 'No measurements yet'; return }
foreach ($file in Get-ChildItem -LiteralPath $runRoot -Filter metrics.json -Recurse | Sort-Object FullName) {
    try { $metric=Read-BenchJson $file.FullName } catch { Write-Warning "Unreadable result (possibly still being written): $($file.FullName)"; continue }
    $metrics.Add($metric)
    $m=$metric.metadata
    $io=$metric['tree_io']
    $row=[ordered]@{
        run_id=$m['run_id'];invocation_id=$m['invocation_id'];parent_build_id=$m['parent_build_id'];pair_id=$m['pair_id'];order=$m['order'];
        profile=$m['profile'];scenario=$m['scenario'];scope=$m['scope'];linker=$m['linker'];fixture_sha256=$m['fixture_sha256'];
        source_sha256=$m['source_sha256'];cache_policy=$m['cache_policy'];output_state=$m['output_state'];
        diagnostic_mode=$m['diagnostic_mode'];sample_ms=$metric['sample_ms'];lld_threads=$m['lld_threads'];warmup=$m['warmup'];
        collector_sha256=$m['collector_sha256'];post_exit_grace_ms=$metric['post_exit_grace_ms'];
        application_link=$m['application_link'];capture=$m['capture'];exit_code=$metric['exit_code'];valid=$metric['valid'];
        wall_s=$metric['wall_s'];tree_wall_s=$metric['tree_wall_s'];cpu_user_s=$metric['cpu_user_s'];cpu_kernel_s=$metric['cpu_kernel_s'];
        tree_cpu_s=$metric['tree_cpu_s'];average_logical_cpus=$metric['average_logical_cpus'];
        peak_tree_commit_bytes=$metric['peak_tree_commit_bytes'];observed_peak_working_set_bytes=$metric['observed_peak_working_set_bytes'];
        tree_page_faults=$metric['tree_page_faults'];observed_peak_threads=$metric['observed_peak_threads'];tree_process_count=$metric['tree_process_count'];
        read_bytes=$(if($io){$io.read_bytes}else{$null});write_bytes=$(if($io){$io.write_bytes}else{$null});
        read_operations=$(if($io){$io.read_operations}else{$null});write_operations=$(if($io){$io.write_operations}else{$null});
        exe_bytes=(Get-ArtifactBytes $metric '.exe');pdb_bytes=(Get-ArtifactBytes $metric '.pdb');
        estimated_other_cpu_s=$metric['estimated_other_cpu_s'];warning_count=$metric['warning_count'];
        started_unix_ms=$metric['started_unix_ms'];command_sha256=$metric['command_sha256'];metrics_path=$file.FullName
    }
    $rows.Add([pscustomobject]$row)
}
$jsonl=[IO.StreamWriter]::new((Join-Path $Campaign 'runs.jsonl'),$false,[Text.UTF8Encoding]::new($false))
try { foreach ($metric in $metrics) { $jsonl.WriteLine(($metric | ConvertTo-Json -Depth 40 -Compress)) } } finally { $jsonl.Dispose() }
$rows | Export-Csv -LiteralPath (Join-Path $Campaign 'runs.csv') -NoTypeInformation -Encoding utf8

$fields=@('wall_s','tree_cpu_s','peak_tree_commit_bytes','observed_peak_working_set_bytes','read_bytes','write_bytes','tree_page_faults','exe_bytes','pdb_bytes')
$eligible=@($rows | Where-Object { !$_.warmup -and !$_.capture -and ($_.scope -eq 'cargo' -or !$_.parent_build_id -or $_.application_link) })
$groupKeys=@('profile','scenario','scope','fixture_sha256','source_sha256','cache_policy','output_state','diagnostic_mode','sample_ms','lld_threads','collector_sha256','post_exit_grace_ms')
$summaries=[Collections.Generic.List[object]]::new()
$comparisons=[Collections.Generic.List[object]]::new()
$report=[Collections.Generic.List[string]]::new()
$report.Add('**Windows linker measurements**')
$report.Add('')
$report.Add("Campaign: $($manifest.campaign_id). Generated $([DateTime]::UtcNow.ToString('o')).")
$report.Add('')
$report.Add('Wall time includes native process creation and stops at the dedicated process-exit waiter. Memory commit is the OS job high-water mark. Resident memory/thread peaks are observed samples. Process I/O is not physical disk traffic. Failed runs, warmups, preparations and traced runs are never pooled into baseline speedups. Raw data and diagnostic runs remain in runs.jsonl/runs.csv.')
$report.Add('')
$report.Add('Memory-mapped reads/writes can bypass the process read/write counters. Large I/O-counter differences between these linkers must not be interpreted as equivalent reductions in physical disk traffic.')
$report.Add('')
foreach ($group in $eligible | Group-Object -Property $groupKeys) {
    $first=$group.Group[0]
    foreach ($backend in @('msvc','lld')) {
        $all=@($group.Group | Where-Object linker -EQ $backend)
        if (!$all.Count) { continue }
        $good=@($all | Where-Object { $_.valid -and $_.exit_code -eq 0 })
        foreach ($field in $fields) {
            $stats=Get-Statistics @($good | ForEach-Object { $_.$field })
            $record=[ordered]@{}
            foreach ($key in $groupKeys) { $record[$key]=$first.$key }
            $record.linker=$backend; $record.metric=$field; $record.attempts=$all.Count; $record.failures=$all.Count-$good.Count
            foreach ($key in $stats.Keys) { $record[$key]=$stats[$key] }
            $summaries.Add([pscustomobject]$record)
        }
    }
    if ($first.diagnostic_mode -ne 'none' -or $first.scenario -notin @('replay','build-edit','build-clean','build-noop')) { continue }
    $pairs=@($group.Group | Where-Object { $_.pair_id } | Group-Object pair_id)
    $matched=[Collections.Generic.List[object]]::new()
    foreach ($pair in $pairs) {
        $lld=@($pair.Group | Where-Object { $_.linker -eq 'lld' -and $_.valid -and $_.exit_code -eq 0 })
        $msvc=@($pair.Group | Where-Object { $_.linker -eq 'msvc' -and $_.valid -and $_.exit_code -eq 0 })
        if ($lld.Count -eq 1 -and $msvc.Count -eq 1) { $matched.Add(@{lld=$lld[0];msvc=$msvc[0]}) }
    }
    if (!$matched.Count) { continue }
    $report.Add(('**{0} / {1} / {2}** — {3} complete pairs; sampling {4} ms; LLD threads {5} (0 means default).' -f $first.profile,$first.scenario,$first.scope,$matched.Count,$first.sample_ms,$first.lld_threads))
    $report.Add('')
    $report.Add('| Metric | MSVC median | LLD median | Median paired reduction | 95% bootstrap interval |')
    $report.Add('| --- | ---: | ---: | ---: | ---: |')
    foreach ($field in $fields) {
        $validPairs=@($matched | Where-Object { $null -ne $_.msvc.$field -and $null -ne $_.lld.$field -and $_.msvc.$field -gt 0 })
        if (!$validPairs.Count) { continue }
        [double[]]$reductions=@($validPairs | ForEach-Object { 100.0*($_.msvc.$field-$_.lld.$field)/$_.msvc.$field })
        $ci=Get-BootstrapInterval $reductions $manifest.seed
        $median=Get-Quantile $reductions 0.5
        $msvcMedian=Get-Quantile @($validPairs | ForEach-Object { $_.msvc.$field }) 0.5
        $lldMedian=Get-Quantile @($validPairs | ForEach-Object { $_.lld.$field }) 0.5
        $comparison=[ordered]@{}
        foreach ($key in $groupKeys) { $comparison[$key]=$first.$key }
        $comparison.metric=$field; $comparison.pairs=$validPairs.Count; $comparison.msvc_median=$msvcMedian; $comparison.lld_median=$lldMedian
        $comparison.paired_reduction_percent=$median; $comparison.ci95_low=$ci[0]; $comparison.ci95_high=$ci[1]
        $comparison.median_paired_speedup=if (@($validPairs | Where-Object { $_.lld.$field -le 0 }).Count) { $null } else { Get-Quantile @($validPairs | ForEach-Object { $_.msvc.$field/$_.lld.$field }) 0.5 }
        $comparisons.Add([pscustomobject]$comparison)
        $unit=if($field.EndsWith('_bytes')){' (MiB)'}else{''}
        $divisor=if($unit){1MB}else{1}
        $interval=if($null -eq $ci[0]){'insufficient pairs'}else{'[{0:N2}; {1:N2}]%' -f $ci[0],$ci[1]}
        $report.Add(('| {0}{1} | {2:N3} | {3:N3} | {4:N2}% | {5} |' -f $field,$unit,($msvcMedian/$divisor),($lldMedian/$divisor),$median,$interval))
    }
    $report.Add('')
}
$summaries | Export-Csv -LiteralPath (Join-Path $Campaign 'summary.csv') -NoTypeInformation -Encoding utf8
$comparisons | Export-Csv -LiteralPath (Join-Path $Campaign 'comparison.csv') -NoTypeInformation -Encoding utf8
Write-BenchJson (Join-Path $Campaign 'comparison.json') @($comparisons)

$calibration=@($rows | Where-Object { $_.scenario -eq 'calibration' -and $_.valid -and $_.exit_code -eq 0 })
if ($calibration.Count) {
    $report.Add('**Collector calibration**')
    $report.Add('')
    foreach ($group in $calibration | Group-Object profile,linker) {
        $effects=[Collections.Generic.List[double]]::new()
        foreach ($pair in $group.Group | Group-Object pair_id) {
            $off=@($pair.Group | Where-Object sample_ms -EQ 0)
            $on=@($pair.Group | Where-Object sample_ms -GT 0)
            if($off.Count -eq 1 -and $on.Count -eq 1) { $effects.Add(100.0*($on[0].wall_s-$off[0].wall_s)/$off[0].wall_s) }
        }
        if ($effects.Count) { $report.Add(('- {0}: median sampling-on wall-time change {1:N2}% across {2} pairs. This includes run noise; inspect raw runs before interpreting it as overhead.' -f $group.Name,(Get-Quantile @($effects) 0.5),$effects.Count)) }
    }
    $report.Add('')
}
$failures=@($rows | Where-Object { !$_.valid -or $_.exit_code -ne 0 })
$report.Add("Retained failed runs: $($failures.Count). Bootstrap resamples whole pairs ($BootstrapSamples replicates, seed $($manifest.seed)). Tail p95 is populated in summary.csv only for groups with at least 30 successful observations.")
$report.Add('')
$report.Add('Compatibility evidence is retained in per-run artifacts.json, smoke-version.log, pe-headers.txt, pe-dependents.txt and pdb-validation.txt. GUI and interactive debugger checks are separate from the timing campaign. Optional ETW files require inspection for provider coverage/event loss before drawing disk or scheduling conclusions.')
$plotGroups=@($eligible | Where-Object { $_.scenario -in @('replay','build-edit','build-clean','build-noop') -and $_.diagnostic_mode -eq 'none' -and $_.valid -and $_.exit_code -eq 0 } | Group-Object -Property $groupKeys)
if ($plotGroups.Count) {
    function Svg-Number([double]$Value) { $Value.ToString('0.###',[Globalization.CultureInfo]::InvariantCulture) }
    $height=50+150*$plotGroups.Count
    $svg=[Collections.Generic.List[string]]::new()
    $svg.Add("<svg xmlns=`"http://www.w3.org/2000/svg`" width=`"820`" height=`"$height`" viewBox=`"0 0 820 $height`">")
    $svg.Add('<rect width="100%" height="100%" fill="white"/><g font-family="Segoe UI, sans-serif" fill="#17212b"><text x="24" y="28" font-size="18">Individual wall times (seconds); vertical bar = median</text>')
    $panel=0
    foreach ($group in $plotGroups) {
        $first=$group.Group[0]
        $top=55+150*$panel
        $label=[Security.SecurityElement]::Escape("$($first.profile) / $($first.scenario) / $($first.scope); sample $($first.sample_ms) ms; drain $($first.post_exit_grace_ms) ms")
        $svg.Add("<text x=`"24`" y=`"$top`" font-size=`"13`">$label</text>")
        $maximum=1.1*($group.Group | Measure-Object wall_s -Maximum).Maximum
        if($maximum -le 0){$maximum=1}
        for($tick=0;$tick -le 4;$tick++) {
            $x=140+155*$tick
            $label=Svg-Number ($maximum*$tick/4)
            $svg.Add("<line x1=`"$x`" x2=`"$x`" y1=`"$($top+12)`" y2=`"$($top+100)`" stroke=`"#e5e7eb`"/><text x=`"$x`" y=`"$($top+116)`" text-anchor=`"middle`" font-size=`"11`">$label</text>")
        }
        $lane=0
        foreach($backend in @('msvc','lld')) {
            $values=@($group.Group | Where-Object linker -EQ $backend | ForEach-Object wall_s)
            $y=$top+37+38*$lane
            $color=if($backend -eq 'msvc'){'#3564bb'}else{'#008978'}
            $svg.Add("<text x=`"24`" y=`"$($y+4)`" font-size=`"12`">$backend (n=$($values.Count))</text>")
            $point=0
            foreach($value in $values) {
                $x=Svg-Number (140+620*$value/$maximum)
                $cy=$y+3*(($point%5)-2)
                $svg.Add("<circle cx=`"$x`" cy=`"$cy`" r=`"3`" fill=`"$color`" fill-opacity=`"0.7`"/>")
                $point++
            }
            if($values.Count) {
                $median=Svg-Number (140+620*(Get-Quantile $values 0.5)/$maximum)
                $svg.Add("<line x1=`"$median`" x2=`"$median`" y1=`"$($y-13)`" y2=`"$($y+13)`" stroke=`"$color`" stroke-width=`"2`"/>")
            }
            $lane++
        }
        $panel++
    }
    $svg.Add('</g></svg>')
    $svg | Set-Content -LiteralPath (Join-Path $Campaign 'wall-times.svg') -Encoding utf8
    $report.Add('')
    $report.Add('![Wall-time distributions](wall-times.svg)')
}
$report | Set-Content -LiteralPath (Join-Path $Campaign 'report.md') -Encoding utf8
Write-Host "Report: $(Join-Path $Campaign 'report.md')"
if ($comparisons.Count) { $comparisons | Where-Object metric -EQ wall_s | Select-Object profile,scenario,scope,pairs,msvc_median,lld_median,paired_reduction_percent | Format-Table -AutoSize | Out-Host }
