[CmdletBinding()]
param([switch]$Force)

$ErrorActionPreference = 'Stop'
$version = '4.1'
$archiveName = 'scrcpy-win64-v4.1.zip'
$url = "https://github.com/Genymobile/scrcpy/releases/download/v$version/$archiveName"
$expectedSha256 = '5b12172b3264b2889f4583ee64752ce832e29bc8b1089dca81093459697165db'
$stateRoot = Join-Path $env:LOCALAPPDATA 'Codex\android-use'
$toolRoot = Join-Path $stateRoot 'tools\scrcpy'
$archive = Join-Path $stateRoot "tools\$archiveName"
$destination = Join-Path $toolRoot 'scrcpy-win64-v4.1'

New-Item -ItemType Directory -Force -Path (Split-Path $archive -Parent) | Out-Null
if ($Force -or !(Test-Path -LiteralPath $archive)) {
  $download = "$archive.download.$PID"
  try {
    Invoke-WebRequest -UseBasicParsing $url -OutFile $download
    $actual = (Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expectedSha256) {
      throw "scrcpy archive SHA-256 mismatch: expected $expectedSha256, got $actual"
    }
    Move-Item -LiteralPath $download -Destination $archive -Force
  } finally {
    Remove-Item -LiteralPath $download -Force -ErrorAction SilentlyContinue
  }
}

$actualArchive = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualArchive -ne $expectedSha256) {
  throw "scrcpy archive SHA-256 mismatch: expected $expectedSha256, got $actualArchive"
}
Expand-Archive -LiteralPath $archive -DestinationPath $toolRoot -Force
$exe = Join-Path $destination 'scrcpy.exe'
if (!(Test-Path -LiteralPath $exe -PathType Leaf)) {
  throw "scrcpy $version archive did not produce $exe"
}
Write-Output ([pscustomobject]@{
  version = $version
  archive = $archive
  sha256 = $actualArchive
  executable = $exe
  virtual_media_devices = $false
} | ConvertTo-Json -Compress)
