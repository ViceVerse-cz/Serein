<#
.SYNOPSIS
Installs or uninstalls Serein for the current Windows user without elevation.
Preserves write permissions for seamless in-app autoupdates.
#>
param(
    [switch]$Uninstall,
    [switch]$Quiet
)

$ErrorActionPreference = 'Stop'
$appName = 'Serein'
$publisher = 'Serein contributors'
$website = 'https://github.com/ViceVerse-cz/Serein'
$installDir = Join-Path $env:LOCALAPPDATA "Programs\$appName"
$uninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$appName"
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) "$appName.lnk"

# Check if Serein is currently running
$running = Get-Process serein -ErrorAction SilentlyContinue
if ($running) {
    if ($Quiet) {
        $running | Stop-Process -Force
    } else {
        throw "Serein is currently running. Please close Serein before running setup."
    }
}

if ($Uninstall) {
    if (Test-Path -LiteralPath $shortcut) {
        Remove-Item -LiteralPath $shortcut -Force
    }
    if (Test-Path -LiteralPath $uninstallKey) {
        Remove-Item -LiteralPath $uninstallKey -Recurse -Force
    }
    $runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
    if ((Get-ItemProperty -LiteralPath $runKey -Name $appName -ErrorAction SilentlyContinue)) {
        Remove-ItemProperty -LiteralPath $runKey -Name $appName -Force
    }
    if (Test-Path -LiteralPath $installDir) {
        Remove-Item -LiteralPath $installDir -Recurse -Force
    }
    if (!$Quiet) {
        Write-Host "Serein was successfully uninstalled."
    }
    return
}

# Install
$distDir = $PSScriptRoot
$executable = Join-Path $distDir 'serein.exe'
if (!(Test-Path -LiteralPath $executable)) {
    throw "serein.exe not found in $distDir. Run this script from the release package directory."
}

[IO.Directory]::CreateDirectory($installDir) | Out-Null

# Copy payload
Copy-Item -Path "$distDir\*" -Destination $installDir -Recurse -Force

# Create Start Menu shortcut with AUMID
$notificationScript = Join-Path $installDir 'install-notifications.ps1'
if (Test-Path -LiteralPath $notificationScript) {
    & $notificationScript -Force
}

# Determine version
$version = '0.1.0'
try {
    $versionInfo = (Get-Item -LiteralPath (Join-Path $installDir 'serein.exe')).VersionInfo.ProductVersion
    if ($versionInfo) { $version = $versionInfo }
} catch {}

# Register in Windows Add/Remove Programs
if (!(Test-Path -LiteralPath $uninstallKey)) {
    New-Item -Path $uninstallKey -Force | Out-Null
}

$uninstallCmd = "powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$installDir\setup.ps1`" -Uninstall"
Set-ItemProperty -LiteralPath $uninstallKey -Name 'DisplayName' -Value $appName
Set-ItemProperty -LiteralPath $uninstallKey -Name 'DisplayVersion' -Value $version
Set-ItemProperty -LiteralPath $uninstallKey -Name 'Publisher' -Value $publisher
Set-ItemProperty -LiteralPath $uninstallKey -Name 'DisplayIcon' -Value "$installDir\serein.exe,0"
Set-ItemProperty -LiteralPath $uninstallKey -Name 'InstallLocation' -Value $installDir
Set-ItemProperty -LiteralPath $uninstallKey -Name 'UninstallString' -Value $uninstallCmd
Set-ItemProperty -LiteralPath $uninstallKey -Name 'QuietUninstallString' -Value "$uninstallCmd -Quiet"
Set-ItemProperty -LiteralPath $uninstallKey -Name 'URLInfoAbout' -Value $website
Set-ItemProperty -LiteralPath $uninstallKey -Name 'NoModify' -Value 1 -Type DWord
Set-ItemProperty -LiteralPath $uninstallKey -Name 'NoRepair' -Value 1 -Type DWord

if (!$Quiet) {
    Write-Host "Serein $version installed successfully to $installDir"
}
