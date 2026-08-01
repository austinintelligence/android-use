[CmdletBinding()]
param(
  [string]$Root,
  [switch]$Check
)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($Root)) { $Root = Split-Path -Parent $PSScriptRoot }
$Root = (Resolve-Path -LiteralPath $Root).Path

$source = @(
  'SKILL.md',
  'agents/openai.yaml'
) + @(Get-ChildItem -LiteralPath (Join-Path $Root 'references') -File |
  Where-Object { $_.Extension -in @('.md', '.json') } |
  ForEach-Object { "references/$($_.Name)" })

$destinations = @(
  (Join-Path $Root 'skills/android-use'),
  (Join-Path $Root 'packages/installer/skill')
)

foreach ($destination in $destinations) {
  if ($Check) {
    foreach ($relative in $source) {
      $from = Join-Path $Root $relative
      $to = Join-Path $destination $relative
      if (-not (Test-Path -LiteralPath $to -PathType Leaf)) { throw "skill payload is missing: $to" }
      $left = (Get-FileHash -LiteralPath $from -Algorithm SHA256).Hash
      $right = (Get-FileHash -LiteralPath $to -Algorithm SHA256).Hash
      if ($left -ne $right) { throw "skill payload is stale: $to" }
    }
    continue
  }

  New-Item -ItemType Directory -Force -Path $destination | Out-Null
  $allowed = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
  foreach ($relative in $source) {
    [void]$allowed.Add($relative.Replace('/', '\'))
    $from = Join-Path $Root $relative
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
else { Write-Output 'synchronized skills/android-use and packages/installer/skill' }
