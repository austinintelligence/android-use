[CmdletBinding()]
param(
  [string]$Root,
  [switch]$Check
)

$ErrorActionPreference = 'Stop'

function Get-Sha256([string]$Path) {
  $stream = [IO.File]::OpenRead($Path)
  try {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-', '') }
    finally { $sha.Dispose() }
  } finally {
    $stream.Dispose()
  }
}

if ([string]::IsNullOrWhiteSpace($Root)) { $Root = Split-Path -Parent $PSScriptRoot }
$Root = (Resolve-Path -LiteralPath $Root).Path

$canonical = Join-Path $Root 'skills/android-use'
if (-not (Test-Path -LiteralPath (Join-Path $canonical 'SKILL.md') -PathType Leaf)) {
  throw "canonical skill is missing: $canonical"
}

$source = @(
  'SKILL.md',
  'agents/openai.yaml'
) + @(Get-ChildItem -LiteralPath (Join-Path $canonical 'references') -File |
  Where-Object { $_.Extension -in @('.md', '.json') } |
  ForEach-Object { "references/$($_.Name)" })

$destinations = @(
  (Join-Path $Root 'packages/installer/skill')
)

foreach ($destination in $destinations) {
  if ($Check) {
    foreach ($relative in $source) {
      $from = Join-Path $canonical $relative
      $to = Join-Path $destination $relative
      if (-not (Test-Path -LiteralPath $to -PathType Leaf)) { throw "skill payload is missing: $to" }
      $left = Get-Sha256 $from
      $right = Get-Sha256 $to
      if ($left -ne $right) { throw "skill payload is stale: $to" }
    }
    continue
  }

  New-Item -ItemType Directory -Force -Path $destination | Out-Null
  $allowed = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
  foreach ($relative in $source) {
    [void]$allowed.Add($relative.Replace('/', '\'))
    $from = Join-Path $canonical $relative
    $to = Join-Path $destination $relative
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $to) | Out-Null
    Copy-Item -LiteralPath $from -Destination $to -Force
  }
  Get-ChildItem -LiteralPath $destination -Recurse -File | ForEach-Object {
    $relative = $_.FullName.Substring($destination.Length).TrimStart('\', '/')
    if (-not $allowed.Contains($relative)) { Remove-Item -LiteralPath $_.FullName -Force }
  }
  Get-ChildItem -LiteralPath $destination -Recurse -Directory |
    Sort-Object FullName -Descending |
    Where-Object { -not (Get-ChildItem -LiteralPath $_.FullName -Force) } |
    ForEach-Object { Remove-Item -LiteralPath $_.FullName -Force }
}

if ($Check) { Write-Output 'public skill payloads are synchronized' }
else { Write-Output 'synchronized packages/installer/skill from skills/android-use' }
