param([int]$ProcessId, [string]$OutputPath)
$ErrorActionPreference='Stop'
$preview=Get-Process -Id $ProcessId
if ($preview.ProcessName -ne 'profile_preview') { throw 'Sample only the synthetic profile preview.' }
Start-Sleep -Seconds 5
$preview.Refresh()
$cpuStart=$preview.TotalProcessorTime.TotalSeconds
$timer=[System.Diagnostics.Stopwatch]::StartNew()
$samples=for ($index=0; $index -lt 20; $index++) {
    Start-Sleep -Milliseconds 500
    $preview.Refresh()
    [pscustomobject]@{ elapsedSeconds=$timer.Elapsed.TotalSeconds; cpuSeconds=$preview.TotalProcessorTime.TotalSeconds; privateBytes=$preview.PrivateMemorySize64; workingSetBytes=$preview.WorkingSet64 }
}
$elapsed=$timer.Elapsed.TotalSeconds
$result=[ordered]@{
    processId=$ProcessId
    warmupSeconds=5
    intervalMilliseconds=500
    durationSeconds=$elapsed
    sampleCount=$samples.Count
    idleCpuPercentOneCore=100*($preview.TotalProcessorTime.TotalSeconds-$cpuStart)/$elapsed
    peakPrivateBytes=($samples | Measure-Object privateBytes -Maximum).Maximum
    settledPrivateBytes=$samples[-1].privateBytes
    peakWorkingSetBytes=($samples | Measure-Object workingSetBytes -Maximum).Maximum
    settledWorkingSetBytes=$samples[-1].workingSetBytes
    samples=$samples
}
$result | ConvertTo-Json -Depth 4 | Set-Content -Encoding UTF8 -LiteralPath $OutputPath
$result.Remove('samples')
$result | ConvertTo-Json -Compress
