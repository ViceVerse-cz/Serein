param([string]$Executable, [string]$Label)
$ErrorActionPreference = 'Stop'
$directory = Split-Path -Parent $MyInvocation.MyCommand.Path
$env:WGPU_BACKEND = 'dx12'
$headless = @()
for ($run = 0; $run -lt 6; $run++) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $output = & $Executable --demo --headless
    if ($LASTEXITCODE -ne 0) { throw "Component run failed: $LASTEXITCODE" }
    $headless += [pscustomobject]@{run=$run;warmup=($run -eq 0);output=$output;wallMs=$watch.Elapsed.TotalMilliseconds}
}
$process = Start-Process -FilePath $Executable -ArgumentList '--demo' -PassThru -WindowStyle Hidden -RedirectStandardOutput (Join-Path $directory "$Label-native.txt") -RedirectStandardError (Join-Path $directory "$Label-stderr.txt")
Start-Sleep -Seconds 8
$process.Refresh()
$lastTime = [DateTime]::UtcNow
$lastCpu = $process.TotalProcessorTime.TotalSeconds
$samples = @()
for ($sample = 0; $sample -lt 20; $sample++) {
    Start-Sleep -Seconds 1
    $process.Refresh()
    if ($process.HasExited) { throw 'Native workload exited before sampling finished' }
    $now = [DateTime]::UtcNow
    $cpu = $process.TotalProcessorTime.TotalSeconds
    $samples += [pscustomobject]@{sample=$sample;cpuOneCorePercent=100*($cpu-$lastCpu)/($now-$lastTime).TotalSeconds;workingSetBytes=$process.WorkingSet64;privateBytes=$process.PrivateMemorySize64}
    $lastCpu = $cpu
    $lastTime = $now
}
if (!$process.WaitForExit(10000)) { $process.Kill(); throw 'Owned workload failed to exit' }
if ($process.ExitCode -ne 0) { throw "Native run failed: $($process.ExitCode)" }
$result = [pscustomobject]@{label=$Label;executable=$Executable;sha256=(Get-FileHash -LiteralPath $Executable).Hash;headless=$headless;native=$samples;nativeOutput=(Get-Content -Raw -LiteralPath (Join-Path $directory "$Label-native.txt"))}
$result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $directory "$Label.json")
$samples | Measure-Object -Property cpuOneCorePercent -Average -Maximum | Select-Object Average,Maximum
$headless | Select-Object run,output
