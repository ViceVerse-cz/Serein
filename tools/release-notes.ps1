#requires -Version 5.1
<#
.SYNOPSIS
  Build the Markdown changelog for a SereinDPI release.

.DESCRIPTION
  Lists the commits between the previous v1.0.* tag and HEAD, groups them by
  Conventional Commit type and links every upstream pull request. The result is
  written to -OutputPath and is used as the GitHub release body, so a release
  never shows up as an empty "Full Changelog" line again.

  Zapret bundle updates (strategies, winws binaries) are called out separately,
  because they are the fork's own reason to publish a build.

.PARAMETER Version
  Version being released, for example 1.0.15 (the workflow sets $env:VERSION).

.PARAMETER OutputPath
  Where to write the Markdown file. Default: release-notes.md

.PARAMETER Repository
  Fork repository used for commit and compare links.

.PARAMETER UpstreamRepository
  Upstream repository used for pull-request links.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Version,
    [string]$OutputPath = 'target/release-notes.md',
    [string]$Repository = 'StepashKA5/SereinDPI',
    [string]$UpstreamRepository = 'ViceVerse-cz/Serein',
    [int]$MaxCommits = 400
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$sectionOrder = @(
    'feat', 'fix', 'perf', 'refactor', 'docs', 'build', 'other'
)
$sectionTitles = @{
    'feat'     = 'Новое'
    'fix'      = 'Исправления'
    'perf'     = 'Производительность'
    'refactor' = 'Рефакторинг'
    'docs'     = 'Документация'
    'build'    = 'Сборка и зависимости'
    'other'    = 'Прочее'
}
$typeSections = @{
    'feat'     = 'feat'
    'fix'      = 'fix'
    'perf'     = 'perf'
    'refactor' = 'refactor'
    'docs'     = 'docs'
    'build'    = 'build'
    'ci'       = 'build'
    'deps'     = 'build'
    'chore'    = 'build'
    'test'     = 'other'
    'style'    = 'other'
    'revert'   = 'other'
}

function Invoke-Git {
    param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Arguments)
    $output = & git @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "git $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
    }
    return @($output)
}

function Test-Ancestor {
    param([string]$Revision)
    & git merge-base --is-ancestor $Revision HEAD 2>$null
    return ($LASTEXITCODE -eq 0)
}

function Get-PreviousTag {
    param([string]$CurrentTag)
    $tags = @(Invoke-Git tag --list 'v1.0.*' --sort=-v:refname)
    foreach ($tag in $tags) {
        if ([string]::IsNullOrWhiteSpace($tag) -or $tag -eq $CurrentTag) { continue }
        if (Test-Ancestor -Revision $tag) { return $tag }
    }
    return $null
}

function Split-Commit {
    param([string]$Line)
    $hash, $subject = $Line -split "`t", 2
    $subject = $subject.Trim()
    $type = 'other'
    $scope = $null
    $text = $subject
    if ($subject -match '^(?<kind>[A-Za-z]+)(\((?<scope>[^)]+)\))?!?:\s*(?<text>.+)$') {
        $type = $Matches['kind'].ToLowerInvariant()
        $scope = $Matches['scope']
        $text = $Matches['text']
    }
    $pullRequest = $null
    if ($text -match '^(?<text>.*?)\s*\(#(?<pr>\d+)\)\s*$') {
        $text = $Matches['text']
        $pullRequest = $Matches['pr']
    }
    [pscustomobject]@{
        Hash        = $hash
        Type        = $type
        Scope       = $scope
        Text        = $text.Trim()
        PullRequest = $pullRequest
    }
}

function Format-Commit($Commit) {
    $line = '- '
    if ($Commit.Scope) { $line += "**$($Commit.Scope):** " }
    $line += $Commit.Text
    if ($Commit.PullRequest) {
        $line += " ([#$($Commit.PullRequest)](https://github.com/$UpstreamRepository/pull/$($Commit.PullRequest)))"
    }
    $line += " · [``$($Commit.Hash.Substring(0, 7))``](https://github.com/$Repository/commit/$($Commit.Hash))"
    return $line
}

$currentTag = "v$Version"
$previousTag = Get-PreviousTag -CurrentTag $currentTag
$range = if ($previousTag) { "$previousTag..HEAD" } else { 'HEAD' }

$raw = @(Invoke-Git log $range '--no-merges' "--max-count=$MaxCommits" '--pretty=format:%H%x09%s')
$commits = @($raw | Where-Object { $_ -and $_.Trim() } | ForEach-Object { Split-Commit -Line $_ })

$buckets = [ordered]@{}
foreach ($section in $sectionOrder) {
    $buckets[$section] = @()
}
foreach ($commit in $commits) {
    $section = if ($typeSections.ContainsKey($commit.Type)) { $typeSections[$commit.Type] } else { 'other' }
    $buckets[$section] += $commit
}

$zapretFiles = @(Invoke-Git log $range '--name-only' '--pretty=format:' '--' 'crates/zapret' 'packaging/windows/zapret' |
    Where-Object { $_ -and $_.Trim() })
$zapretChanged = $zapretFiles.Count -gt 0

$lines = @()
$builtAt = Get-Date -Format 'dd.MM.yyyy'
$lines += ("Сборка **$Version** от $builtAt.")
$lines += ('')

$featureCount = $buckets['feat'].Count
$fixCount = $buckets['fix'].Count
$summary = @()
if ($featureCount -gt 0) { $summary += "Новых возможностей: $featureCount" }
if ($fixCount -gt 0) { $summary += "Исправлений: $fixCount" }
if ($summary.Count -gt 0) {
    $lines += ($summary -join ' · ')
    $lines += ('')
}

if ($commits.Count -eq 0) {
    $lines += ('Новых коммитов нет: сборка пересобрана без изменений в коде.')
    $lines += ('')
}

foreach ($section in $sectionOrder) {
    $items = $buckets[$section]
    if ($items.Count -eq 0) { continue }
    $lines += ("### $($sectionTitles[$section])")
    $lines += ('')
    foreach ($item in $items) {
        $lines += (Format-Commit -Commit $item)
    }
    $lines += ('')
}

if ($zapretChanged) {
    $strategiesPath = 'crates/zapret/data/strategies.json'
    $strategyLine = 'Обновлён встроенный обход DPI.'
    if (Test-Path -LiteralPath $strategiesPath) {
        $catalog = Get-Content -LiteralPath $strategiesPath -Raw -Encoding UTF8 | ConvertFrom-Json
        $count = @($catalog.strategies).Count
        $source = $catalog.source
        $strategyLine = "Обновлён встроенный обход DPI: $count стратегий из ``$source``."
    }
    $lines += ('### Обход DPI (zapret)')
    $lines += ('')
    $lines += ("- $strategyLine")
    if ($zapretFiles -match 'winws\.exe') {
        $lines += ('- Обновлены бинарники winws и вспомогательные списки.')
    }
    $lines += ('')
}

$lines += ('---')
if ($previousTag) {
    $lines += ("**Полный список изменений:** [$previousTag...$currentTag](https://github.com/$Repository/compare/$previousTag...$currentTag)")
} else {
    $lines += ("**Полный список изменений:** [история коммитов](https://github.com/$Repository/commits/main)")
}
$lines += ('Установщик и портативная сборка — во вложениях релиза.')

$text = ($lines -join "`n") + "`n"
$directory = Split-Path -Parent $OutputPath
if ($directory -and -not (Test-Path -LiteralPath $directory)) {
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
}
$fullPath = if ([IO.Path]::IsPathRooted($OutputPath)) { $OutputPath } else { Join-Path (Get-Location) $OutputPath }
[IO.File]::WriteAllText($fullPath, $text, (New-Object Text.UTF8Encoding($false)))
Write-Host "Wrote $OutputPath ($($commits.Count) commits, previous tag: $previousTag)"
Write-Host $text
