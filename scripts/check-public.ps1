[CmdletBinding()]
param([string]$Root)

$ErrorActionPreference = 'Stop'
if ([string]::IsNullOrWhiteSpace($Root)) { $Root = Split-Path -Parent $PSScriptRoot }
$Root = (Resolve-Path -LiteralPath $Root).Path

$files = @(git -C $Root ls-files --cached --others --exclude-standard |
  Where-Object { $_ -and $_ -notmatch '^(artifacts|target|android/aubridge/(?:[^/]+/)?build|android/aubridge/\.gradle)/' })
$rules = @(
  @{ name = 'long numeric identity'; pattern = '\b\d{12,20}\b' },
  @{ name = 'absolute Windows user path'; pattern = '(?i)\b[A-Z]:\\Users\\' },
  @{ name = 'legacy canvas listener'; pattern = '0\.0\.0\.0:8765|canvas\.html' },
  @{ name = 'embedded credential'; pattern = '(?i)(bridge_token|access[_-]?token)\s*[=:]\s*[A-Za-z0-9+/=_-]{24,}' }
)
$violations = [System.Collections.Generic.List[object]]::new()
foreach ($relative in $files) {
  $path = Join-Path $Root ($relative -replace '/', '\')
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { continue }
  try { $text = [IO.File]::ReadAllText($path) } catch { continue }
  if ($text.IndexOf([char]0) -ge 0) { continue }
  foreach ($rule in $rules) {
    if ([regex]::IsMatch($text, $rule.pattern)) {
      $violations.Add([pscustomobject]@{ file = $relative; rule = $rule.name })
    }
  }
}
if ($violations.Count -gt 0) {
  $violations | ConvertTo-Json -Compress
  throw "public scan failed with $($violations.Count) finding(s)"
}
Write-Output "public scan passed files=$($files.Count)"
