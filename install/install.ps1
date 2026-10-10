# Install au.exe from a GitHub release (the latest, or ANDROID_USE_VERSION; x86_64 and arm64).
# The archive is verified against SHA256SUMS before anything is extracted.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Repo = 'https://github.com/austinintelligence/android-use'
$InstallDir = if ($env:AU_INSTALL_DIR) { $env:AU_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'AndroidUse\bin' }

# The newest release unless ANDROID_USE_VERSION pins one (for example 1.2.0).
if ($env:ANDROID_USE_VERSION) {
  $Version = $env:ANDROID_USE_VERSION.TrimStart('v')
} else {
  $Version = (Invoke-RestMethod -Uri 'https://api.github.com/repos/austinintelligence/android-use/releases/latest').tag_name.TrimStart('v')
  if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw 'Could not find the latest release. Set ANDROID_USE_VERSION.' }
}
$Base = "$Repo/releases/download/v$Version"

$Arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
$Asset = switch ($Arch) {
  'X64' { "android-use-v$Version-windows-x86_64.zip" }
  'Arm64' { "android-use-v$Version-windows-aarch64.zip" }
  default { throw "Unsupported platform: Windows $Arch. Supported: x86_64 and arm64." }
}

$Tmp = Join-Path ([IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $Tmp | Out-Null
try {
  Invoke-WebRequest -Uri "$Base/$Asset" -OutFile (Join-Path $Tmp $Asset)
  Invoke-WebRequest -Uri "$Base/SHA256SUMS" -OutFile (Join-Path $Tmp 'SHA256SUMS')

  # Verify before extracting anything from the archive.
  $Expected = Get-Content (Join-Path $Tmp 'SHA256SUMS') | ForEach-Object {
    $parts = $_ -split '\s+'
    if ($parts.Count -ge 2 -and $parts[1].TrimStart('*') -eq $Asset) { $parts[0] }
  } | Select-Object -First 1
  if (-not $Expected) { throw "SHA256SUMS has no entry for $Asset. Aborting." }
  $Actual = (Get-FileHash -Algorithm SHA256 (Join-Path $Tmp $Asset)).Hash
  if ($Actual -ine $Expected) { throw "SHA256 mismatch for $Asset (expected $Expected, got $Actual). Aborting." }

  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $Zip = [IO.Compression.ZipFile]::OpenRead((Join-Path $Tmp $Asset))
  try {
    [IO.Compression.ZipFileExtensions]::ExtractToFile($Zip.GetEntry('au.exe'), (Join-Path $Tmp 'au.exe'), $true)
  } finally {
    $Zip.Dispose()
  }

  New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
  Copy-Item (Join-Path $Tmp 'au.exe') (Join-Path $InstallDir 'au.exe') -Force
  Write-Output "Installed au to $InstallDir. Next: au setup"
  if (-not (($env:Path -split ';') -contains $InstallDir)) {
    Write-Output "Add $InstallDir to your PATH to run au from anywhere."
  }
} finally {
  Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
