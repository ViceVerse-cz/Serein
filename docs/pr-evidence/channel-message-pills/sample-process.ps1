param(
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$Output
)
$ErrorActionPreference = 'Stop'
$preview = Start-Process -FilePath $Executable -ArgumentList '--demo', '--page=channel-links', '--width=1120', '--height=900', '--interactive' -WindowStyle Hidden -PassThru
try {
    Start-Sleep -Seconds 5
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $samples = @(for ($index = 0; $index -le 20; $index++) {
        $preview.Refresh()
        if ($preview.HasExited) { throw 'Preview exited during measurement' }
        [pscustomobject]@{
            seconds = $watch.Elapsed.TotalSeconds
            cpu_seconds = $preview.TotalProcessorTime.TotalSeconds
            working_set_bytes = $preview.WorkingSet64
            private_bytes = $preview.PrivateMemorySize64
        }
        if ($index -lt 20) { Start-Sleep -Seconds 1 }
    })
    $samples | ConvertTo-Json | Set-Content -LiteralPath $Output
} finally {
    if (-not $preview.HasExited) { Stop-Process -Id $preview.Id }
}
