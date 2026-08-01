[CmdletBinding()]
param(
  [string]$Version = '1.0.0',
  [Parameter(Mandatory = $true)][string]$ReleaseBaseUrl,
  [Parameter(Mandatory = $true)][string]$HostBinary,
  [Parameter(Mandatory = $true)][string]$HelperApk,
  [Parameter(Mandatory = $true)][string]$Output
)

$ErrorActionPreference = 'Stop'
function Asset([string]$Path, [string]$Url) {
  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "release asset missing: $Path" }
  [ordered]@{
    url = "$ReleaseBaseUrl/$Url"
    bytes = [int64](Get-Item -LiteralPath $Path).Length
    sha256 = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
  }
}
$manifest = [ordered]@{
  schema = 1
  product = 'android-use'
  version = $Version
  protocol_version = 1
  assets = [ordered]@{
    host_windows_x64 = Asset $HostBinary 'au-windows-x64.exe'
    helper_apk = Asset $HelperApk 'dev.codex.aubridge.apk'
  }
}
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Output) | Out-Null
$manifest | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $Output -Encoding UTF8
Write-Output "manifest=$Output"
