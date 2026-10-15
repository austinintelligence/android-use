# Install au.exe from a GitHub release (the latest, or ANDROID_USE_VERSION; x86_64 and arm64).
# The archive is verified against SHA256SUMS, and against its build attestation when gh is installed (always,
# with AU_REQUIRE_ATTESTATION=1), before anything is extracted. The install is one rename; a failure leaves the
# previous au.exe untouched.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
# Windows PowerShell 5.1 defaults to older protocols; this limits it to TLS 1.2. PowerShell 7 already negotiates TLS 1.2+.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Repo = 'https://github.com/austinintelligence/android-use'
$InstallDir = if ($env:AU_INSTALL_DIR) { $env:AU_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'AndroidUse\bin' }

# The newest release unless ANDROID_USE_VERSION pins one (for example 1.2.0 or v1.2.0).
if ($env:ANDROID_USE_VERSION) {
  $Version = $env:ANDROID_USE_VERSION -replace '^v', ''
  if ($Version -notmatch '^\d+\.\d+\.\d+$') { throw 'ANDROID_USE_VERSION must look like 1.2.0 or v1.2.0. Aborting.' }
} else {
  # Follow the releases/latest redirect, as install.sh does, instead of the rate-limited GitHub API.
  $Final = (Invoke-WebRequest -Uri "$Repo/releases/latest" -Method Head -UseBasicParsing).BaseResponse
  $Url = if ($Final.ResponseUri) { $Final.ResponseUri.AbsoluteUri } else { $Final.RequestMessage.RequestUri.AbsoluteUri }
  if ($Url -notmatch '^https://github\.com/austinintelligence/android-use/releases/tag/v(\d+\.\d+\.\d+)$') { throw 'Could not find the latest release over https. Set ANDROID_USE_VERSION.' }
  $Version = $Matches[1]
}
$Base = "$Repo/releases/download/v$Version"

$Arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
$Asset = switch ($Arch) {
  'X64' { "android-use-v$Version-windows-x86_64.zip" }
  'Arm64' { "android-use-v$Version-windows-aarch64.zip" }
  default { throw "Unsupported platform: Windows $Arch. Supported: x86_64 and arm64." }
}

# Stage inside the install directory so the final move is a rename on one volume.
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$Tmp = Join-Path $InstallDir (".au-install-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $Tmp | Out-Null
try {
  Invoke-WebRequest -UseBasicParsing -Uri "$Base/$Asset" -OutFile (Join-Path $Tmp $Asset)
  Invoke-WebRequest -UseBasicParsing -Uri "$Base/SHA256SUMS" -OutFile (Join-Path $Tmp 'SHA256SUMS')

  # Verify before extracting anything from the archive.
  $Expected = Get-Content (Join-Path $Tmp 'SHA256SUMS') | ForEach-Object {
    $parts = $_ -split '\s+'
    if ($parts.Count -ge 2 -and $parts[1].TrimStart('*') -eq $Asset) { $parts[0] }
  } | Select-Object -First 1
  if (-not $Expected) { throw "SHA256SUMS has no entry for $Asset. Aborting." }
  $Actual = (Get-FileHash -Algorithm SHA256 (Join-Path $Tmp $Asset)).Hash
  if ($Actual -ine $Expected) { throw "SHA256 mismatch for $Asset (expected $Expected, got $Actual). Aborting." }

  # The attestation proves the archive was built by this repository's release workflow. Only a logged-in gh can check it.
  if ((Get-Command gh -ErrorAction SilentlyContinue) -and (gh auth status 2>&1 | Out-Null; $LASTEXITCODE -eq 0)) {
    gh attestation verify (Join-Path $Tmp $Asset) --repo austinintelligence/android-use --signer-workflow austinintelligence/android-use/.github/workflows/release.yml | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Attestation check failed for $Asset. Aborting." }
  } elseif ($env:AU_REQUIRE_ATTESTATION -eq '1') {
    throw 'AU_REQUIRE_ATTESTATION=1 needs the GitHub CLI (gh), logged in with gh auth login, to verify the attestation (cosign alone cannot). Aborting.'
  } else {
    Write-Warning 'Skipping the attestation check: install gh and run gh auth login to enable it (SHA256SUMS was verified).'
  }

  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $Zip = [IO.Compression.ZipFile]::OpenRead((Join-Path $Tmp $Asset))
  try {
    [IO.Compression.ZipFileExtensions]::ExtractToFile($Zip.GetEntry('au.exe'), (Join-Path $Tmp 'au.exe'), $true)
  } finally {
    $Zip.Dispose()
  }

  Move-Item -Force (Join-Path $Tmp 'au.exe') (Join-Path $InstallDir 'au.exe')
  Write-Output "Installed au to $InstallDir. Next: au setup"
  if (-not (($env:Path -split ';') -contains $InstallDir)) {
    Write-Output "Add $InstallDir to your PATH to run au from anywhere."
  }
} finally {
  Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
