#requires -Version 5.1
<#
.SYNOPSIS
Samples one prebuilt, synthetic Friends Online process; never discovers a signed-in client.
.DESCRIPTION
Build with --features demo first. Run the same script against both prebuilt revisions, with
identical arguments, display settings, and no keyboard/pointer interaction during sampling.
The app selects Friends Online and focuses Search. It emits bounded start/complete records
around a window that excludes warmup. CPU and memory are sampled between those records.
Five alternating before/after pairs are recommended. Debug and release are separate workloads.
Callback wall-time buckets cover Desktop logic through UI, not tessellation or presentation.
Results are JSON on stdout. A disturbed/unfocused run returns valid=false and must be discarded.
.EXAMPLE
powershell -NoProfile -File scripts/frame-sample.ps1 -Executable E:\build\debug\serein.exe -Revision abc123 -Profile debug
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $Executable,
    [Parameter(Mandatory = $true)] [string] $Revision,
    [Parameter(Mandatory = $true)] [ValidateSet('debug', 'release')] [string] $Profile,
    [ValidateRange(1, 600)] [int] $WarmupSeconds = 8,
    [ValidateRange(1, 600)] [int] $SampleSeconds = 15,
    [ValidateRange(10, 1000)] [int] $IntervalMilliseconds = 100
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).ProviderPath
if (-not [IO.File]::Exists($resolvedExecutable) -or [IO.Path]::GetExtension($resolvedExecutable) -ne '.exe') {
    throw 'Executable must be the exact prebuilt serein.exe path (built with --features demo).'
}
$executableHash = (Get-FileHash -LiteralPath $resolvedExecutable -Algorithm SHA256).Hash
$startInfo = New-Object Diagnostics.ProcessStartInfo
$startInfo.FileName = $resolvedExecutable
$startInfo.WorkingDirectory = [IO.Path]::GetDirectoryName($resolvedExecutable)
$startInfo.Arguments = "--demo --demo-friends --demo-frame-sample=$WarmupSeconds,$SampleSeconds"
$startInfo.UseShellExecute = $false
$startInfo.CreateNoWindow = $true
$startInfo.RedirectStandardOutput = $true
$sampleProcess = New-Object Diagnostics.Process
$sampleProcess.StartInfo = $startInfo
$started = $false
try {
    $started = $sampleProcess.Start()
    if (-not $started) { throw 'Could not start the synthetic process.' }
    $deadline = [Diagnostics.Stopwatch]::StartNew()
    $lineTask = $sampleProcess.StandardOutput.ReadLineAsync()
    $sampleClock = $null
    $startRecord = $null
    $summary = $null
    $cpuStart = 0.0
    $peakWorkingSet = 0L
    $peakPrivate = 0L
    $memorySamples = 0
    $outputLines = 0
    while ($null -eq $summary) {
        if ($deadline.Elapsed.TotalSeconds -gt ($WarmupSeconds + $SampleSeconds + 60)) {
            throw "No complete sample in time (start_marker_received=$($null -ne $sampleClock)); verify the demo-capable binary and that its native window can render and hold focus."
        }
        if ($lineTask.Wait($IntervalMilliseconds)) {
            $line = $lineTask.Result
            if ($null -eq $line) { throw 'Synthetic process ended before reporting a complete sample.' }
            $outputLines++
            if ($line.Length -gt 4096 -or $outputLines -gt 128) {
                throw 'Unexpectedly large diagnostic output; refusing the sample.'
            }
            if ($line.StartsWith('{') -and $line.Contains('"serein_frame_sample"')) {
                $record = $line | ConvertFrom-Json
                if ($record.serein_frame_sample -eq 'start') {
                    if ($null -ne $sampleClock) { throw 'Duplicate sample-start marker.' }
                    $startRecord = $record
                    $sampleProcess.Refresh()
                    $cpuStart = $sampleProcess.TotalProcessorTime.TotalSeconds
                    $sampleClock = [Diagnostics.Stopwatch]::StartNew()
                } elseif ($record.serein_frame_sample -eq 'complete') {
                    if ($null -eq $sampleClock) { throw 'Sample-complete marker without a start marker.' }
                    $summary = $record
                }
            }
            if ($null -eq $summary) { $lineTask = $sampleProcess.StandardOutput.ReadLineAsync() }
        }
        if ($sampleProcess.HasExited) { throw "Synthetic process exited early ($($sampleProcess.ExitCode))." }
        if ($null -ne $sampleClock) {
            $sampleProcess.Refresh()
            $peakWorkingSet = [Math]::Max($peakWorkingSet, $sampleProcess.WorkingSet64)
            $peakPrivate = [Math]::Max($peakPrivate, $sampleProcess.PrivateMemorySize64)
            $memorySamples++
        }
    }
    $sampleProcess.Refresh()
    $cpuEnd = $sampleProcess.TotalProcessorTime.TotalSeconds
    $sampleClock.Stop()
    $valid = $summary.callbacks -gt 0 -and
        $summary.callbacks -eq $summary.without_input -and
        $summary.callbacks -eq $summary.viewport_focused -and
        $summary.callbacks -eq $summary.search_focused -and
        $null -ne $summary.viewport_size -and
        ($summary.viewport_size -join ',') -eq ($startRecord.viewport_size -join ',') -and
        $summary.pixels_per_point -eq $startRecord.pixels_per_point -and
        [Math]::Abs($summary.elapsed_ms - $sampleClock.Elapsed.TotalMilliseconds) -le (2 * $IntervalMilliseconds)
    [ordered]@{
        valid = $valid
        rejection = $(if ($valid) { $null } else { 'Input, lost focus, changed/unknown viewport, missing callbacks, or delayed marker receipt; discard this run.' })
        executable = $resolvedExecutable
        sha256 = $executableHash
        revision = $Revision
        profile = $Profile
        process_id = $sampleProcess.Id
        scenario = 'synthetic --demo-friends, Online, empty Search focused'
        warmup_seconds = $WarmupSeconds
        requested_sample_seconds = $SampleSeconds
        measured_sample_seconds = $sampleClock.Elapsed.TotalSeconds
        interval_ms = $IntervalMilliseconds
        cpu_percent_one_core = 100 * ($cpuEnd - $cpuStart) / $sampleClock.Elapsed.TotalSeconds
        working_set_settled_bytes = $sampleProcess.WorkingSet64
        working_set_peak_sampled_bytes = $peakWorkingSet
        private_settled_bytes = $sampleProcess.PrivateMemorySize64
        private_peak_sampled_bytes = $peakPrivate
        memory_samples = $memorySamples
        frame_window_start = $startRecord
        frames = $summary
    } | ConvertTo-Json -Depth 4
} finally {
    # Only this invocation's process object is ever closed or killed; no name/PID discovery.
    try {
        if ($started -and -not $sampleProcess.HasExited) {
            $null = $sampleProcess.CloseMainWindow()
            if (-not $sampleProcess.WaitForExit(5000)) {
                $sampleProcess.Kill()
                if (-not $sampleProcess.WaitForExit(5000)) {
                    throw "Synthetic process $($sampleProcess.Id) did not exit after termination."
                }
            }
        }
    } finally {
        $sampleProcess.Dispose()
    }
}
