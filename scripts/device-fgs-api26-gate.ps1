#requires -Version 5.1

[CmdletBinding()]
param(
  [switch]$Execute,
  [string]$AuPath,
  [string]$HelperApk,
  [string]$InstrumentationHelperApk,
  [string]$TestApk,
  [string]$FixtureApk,
  [string]$ExpectedHelperSha256 = 'a60f80559d68a2b3921502ab8fb00ee7058344b27e3c631efe58f4bf8d449e48',
  [string]$ExpectedTestSha256 = '395ab5c6123311eafdf9fb06d7358b462f7b58ef215d353b05325f13e9b14a58',
  [string]$ExpectedFixtureSha256 = '969289977b3983c0b0176897df6de26bd405b6b6a3e4a225f7a005685d99fdf8'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptRoot
$defaultApkRoot = Join-Path $repoRoot 'android\aubridge'
if ([string]::IsNullOrWhiteSpace($HelperApk)) { $HelperApk = Join-Path $defaultApkRoot 'app\build\outputs\apk\release\app-release.apk' }
if ([string]::IsNullOrWhiteSpace($InstrumentationHelperApk)) { $InstrumentationHelperApk = Join-Path $defaultApkRoot 'app\build\outputs\apk\debug\app-debug.apk' }
if ([string]::IsNullOrWhiteSpace($TestApk)) { $TestApk = Join-Path $defaultApkRoot 'app\build\outputs\apk\androidTest\debug\app-debug-androidTest.apk' }
if ([string]::IsNullOrWhiteSpace($FixtureApk)) { $FixtureApk = Join-Path $defaultApkRoot 'fixture\build\outputs\apk\release\fixture-release.apk' }

$helperPackage = 'dev.codex.aubridge'
$testPackage = 'dev.codex.aubridge.test'
$fixturePackage = 'dev.codex.aubench'
$accessibilityComponent = 'dev.codex.aubridge/dev.codex.aubridge.AubridgeAccessibilityService'
$accessibilityShortComponent = 'dev.codex.aubridge/.AubridgeAccessibilityService'
$repoRoot = [IO.Path]::GetFullPath($repoRoot)
$stateRoot = Join-Path $env:LOCALAPPDATA 'Codex\android-use'
$configPath = Join-Path $stateRoot 'config.json'
$runStamp = [DateTimeOffset]::Now.ToString('yyyyMMdd-HHmmss')
$privateRoot = Join-Path $stateRoot "artifacts\device-gates\$runStamp"
$rawLog = Join-Path $privateRoot 'raw.log'
$privateSummary = Join-Path $privateRoot 'summary.redacted.json'
$script:baselineEndpoint = $null
$script:baselineTransportId = $null
$script:preForeground = $null
$script:phase = 'initializing'
$script:phaseResults = [Collections.Generic.List[object]]::new()
$script:failures = [Collections.Generic.List[string]]::new()
$script:cleanupNotes = [Collections.Generic.List[string]]::new()
$script:mutationStarted = $false

New-Item -ItemType Directory -Force -Path $privateRoot | Out-Null

function Add-PrivateLog {
  param([string]$Label, [string]$Text)
  Add-Content -LiteralPath $rawLog -Encoding UTF8 -Value @(
    "===== $([DateTimeOffset]::Now.ToString('o')) $Label ====="
    $Text
  )
}

function Invoke-Private {
  param(
    [Parameter(Mandatory = $true)][string]$File,
    [Parameter(Mandatory = $true)][string[]]$Arguments,
    [Parameter(Mandatory = $true)][string]$Label,
    [switch]$AllowFailure
  )
  $previousPreference = $ErrorActionPreference
  $ErrorActionPreference = 'Continue'
  try {
    $lines = @(& $File @Arguments 2>&1 | ForEach-Object { $_.ToString() })
    $exitCode = $LASTEXITCODE
  } finally {
    $ErrorActionPreference = $previousPreference
  }
  $text = $lines -join [Environment]::NewLine
  Add-PrivateLog -Label $Label -Text $text
  if ($exitCode -ne 0 -and -not $AllowFailure) {
    throw "$Label failed with exit code $exitCode; inspect the private raw log"
  }
  [pscustomobject]@{ ExitCode = $exitCode; Text = $text; Lines = $lines }
}

function Resolve-Au {
  if (-not [string]::IsNullOrWhiteSpace($AuPath)) {
    $resolved = Resolve-Path -LiteralPath $AuPath -ErrorAction Stop
    if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) { throw "au binary is missing: $resolved" }
    return $resolved.Path
  }
  $candidates = @(
    (Join-Path $repoRoot 'target\release\au.exe'),
    (Join-Path $stateRoot 'bin\au.exe'),
    (Get-Command au.exe -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty Source)
  ) | Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Leaf) }
  if (@($candidates).Count -eq 0) { throw 'au.exe was not found' }
  @($candidates)[0]
}

function Resolve-Adb {
  param([object]$Config)
  $configured = if ($Config.PSObject.Properties.Name -contains 'adb_path') { [string]$Config.adb_path } else { '' }
  $candidates = @(
    $configured,
    (Join-Path $env:LOCALAPPDATA 'Android\Sdk\platform-tools\adb.exe'),
    (Get-Command adb.exe -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty Source)
  ) | Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Leaf) }
  if (@($candidates).Count -eq 0) { throw 'adb.exe was not found' }
  @($candidates)[0]
}

function Resolve-ApkSigner {
  $buildToolsRoot = Join-Path $env:LOCALAPPDATA 'Android\Sdk\build-tools'
  $candidate = Get-ChildItem -LiteralPath $buildToolsRoot -Directory -ErrorAction SilentlyContinue |
    Sort-Object { try { [version]$_.Name } catch { [version]'0.0' } } -Descending |
    ForEach-Object { Join-Path $_.FullName 'apksigner.bat' } |
    Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } |
    Select-Object -First 1
  if (-not $candidate) { throw 'apksigner.bat was not found' }
  $candidate
}

function Get-Sha256 {
  param([string]$Path)
  (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Assert-Artifact {
  param([string]$Name, [string]$Path, [string]$Expected)
  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "$Name artifact is missing" }
  $actual = Get-Sha256 -Path $Path
  if ($actual -ne $Expected.ToLowerInvariant()) { throw "$Name SHA-256 mismatch" }
  [pscustomobject]@{ name = $Name; sha256 = $actual; bytes = (Get-Item -LiteralPath $Path).Length }
}

function Get-SignerDigest {
  param([string]$Apk, [string]$Label)
  $result = Invoke-Private -File $script:apkSigner -Arguments @('verify', '--verbose', '--print-certs', $Apk) -Label $Label
  $match = [regex]::Match($result.Text, 'Signer #1 certificate SHA-256 digest:\s*([0-9a-fA-F]+)')
  if (-not $match.Success) { throw "$Label did not report a signer digest" }
  $match.Groups[1].Value.ToLowerInvariant()
}

function Get-IdentityFingerprint {
  param([string]$Value)
  $sha = [Security.Cryptography.SHA256]::Create()
  try {
    $bytes = [Text.Encoding]::UTF8.GetBytes($Value)
    ([BitConverter]::ToString($sha.ComputeHash($bytes))).Replace('-', '').Substring(0, 12).ToLowerInvariant()
  } finally {
    $sha.Dispose()
  }
}

function Assert-ExactIdentity {
  param([string]$Reason)
  $inventory = Invoke-Private -File $script:adb -Arguments @('devices', '-l') -Label "identity-$Reason-devices"
  $online = @()
  foreach ($line in $inventory.Lines) {
    if ($line -match '^(\S+)\s+device\s*(.*)$') {
      $transportMatch = [regex]::Match($Matches[2], '(?:^|\s)transport_id:(\S+)')
      $online += [pscustomobject]@{
        Endpoint = $Matches[1]
        TransportId = if ($transportMatch.Success) { $transportMatch.Groups[1].Value } else { '' }
      }
    }
  }
  if ($online.Count -ne 1) { throw "identity gate $Reason requires exactly one online transport; found $($online.Count)" }
  $candidate = $online[0]
  if (-not $candidate.TransportId) { throw "identity gate $Reason did not receive a transport id" }
  $identity = Invoke-Private -File $script:adb -Arguments @('-s', $candidate.Endpoint, 'shell', 'getprop', 'ro.serialno') -Label "identity-$Reason-android"
  if ($identity.Text.Trim() -ne $script:enrolledSerial) { throw "identity gate $Reason rejected the Android hardware identity" }
  if ($script:baselineEndpoint) {
    if ($candidate.Endpoint -ne $script:baselineEndpoint -or $candidate.TransportId -ne $script:baselineTransportId) {
      throw "identity gate $Reason rejected a transport change"
    }
  } else {
    $script:baselineEndpoint = $candidate.Endpoint
    $script:baselineTransportId = $candidate.TransportId
  }
  $candidate.Endpoint
}

function Invoke-Adb {
  param([string[]]$Arguments, [string]$Label, [switch]$AllowFailure)
  Invoke-Private -File $script:adb -Arguments (@('-s', $script:baselineEndpoint) + $Arguments) -Label $Label -AllowFailure:$AllowFailure
}

function Invoke-Au {
  param([string[]]$Arguments, [string]$Label, [switch]$AllowFailure)
  Invoke-Private -File $script:au -Arguments (@('-s', $script:baselineEndpoint) + $Arguments) -Label $Label -AllowFailure:$AllowFailure
}

function Invoke-AuSnapshotRetry {
  param([string]$Label)
  for ($attempt = 0; $attempt -lt 20; $attempt++) {
    $result = Invoke-Au -Arguments @('--timeout', '30000', '-w', 'ui', 'snap', '--compact', '--frontier') -Label "$Label-$attempt" -AllowFailure
    if ($result.ExitCode -eq 0) { return $result }
    if ($result.Text -notmatch 'E_UI|no active accessibility window') {
      throw "$Label failed with a non-transient accessibility error"
    }
    Start-Sleep -Milliseconds 250
  }
  throw "$Label did not observe an accessibility window after the bounded launch wait"
}

function Get-Setting {
  param([string]$Namespace, [string]$Key, [string]$Label)
  (Invoke-Adb -Arguments @('shell', 'settings', 'get', $Namespace, $Key) -Label $Label).Text.Trim()
}

function Get-AccessibilityState {
  [pscustomobject]@{
    entries = Get-Setting -Namespace 'secure' -Key 'enabled_accessibility_services' -Label 'accessibility-entries'
    enabled = Get-Setting -Namespace 'secure' -Key 'accessibility_enabled' -Label 'accessibility-enabled'
  }
}

function Get-ForegroundComponent {
  param([string]$Label)
  $result = Invoke-Adb -Arguments @('shell', 'dumpsys', 'activity', 'activities') -Label $Label
  $match = [regex]::Match($result.Text, 'ActivityRecord\{[^\s]+\s+u\d+\s+([A-Za-z0-9._]+/[A-Za-z0-9.$_]+)\}')
  if ($match.Success) { $match.Groups[1].Value } else { $null }
}

function Restore-ForegroundComponent {
  if (-not $script:preForeground) { return }
  Invoke-Adb -Arguments @('shell', 'am', 'start', '-W', '-n', $script:preForeground) -Label 'restore-foreground' -AllowFailure | Out-Null
}

function Set-TestAccessibility {
  Assert-ExactIdentity -Reason 'enable-au-accessibility' | Out-Null
  $current = Get-AccessibilityState
  $entries = @()
  if ($current.entries -and $current.entries -ne 'null') {
    $entries = @($current.entries.Split(':') | Where-Object { $_ })
  }
  $hasAu = @($entries | Where-Object { $_ -eq $accessibilityComponent -or $_ -eq $accessibilityShortComponent }).Count -gt 0
  if (-not $hasAu) { $entries += $accessibilityComponent }
  Invoke-Adb -Arguments @('shell', 'settings', 'put', 'secure', 'enabled_accessibility_services', ($entries -join ':')) -Label 'enable-au-accessibility-entries' | Out-Null
  Invoke-Adb -Arguments @('shell', 'settings', 'put', 'secure', 'accessibility_enabled', '1') -Label 'enable-au-accessibility-master' | Out-Null
}

function Restore-Accessibility {
  param([object]$State)
  Assert-ExactIdentity -Reason 'restore-accessibility' | Out-Null
  if (-not $State.entries -or $State.entries -eq 'null') {
    Invoke-Adb -Arguments @('shell', 'settings', 'delete', 'secure', 'enabled_accessibility_services') -Label 'restore-accessibility-entries-delete' -AllowFailure | Out-Null
  } else {
    Invoke-Adb -Arguments @('shell', 'settings', 'put', 'secure', 'enabled_accessibility_services', [string]$State.entries) -Label 'restore-accessibility-entries' | Out-Null
  }
  Invoke-Adb -Arguments @('shell', 'settings', 'put', 'secure', 'accessibility_enabled', [string]$State.enabled) -Label 'restore-accessibility-enabled' | Out-Null
}

function Get-ForwardState {
  param([string]$Label)
  $result = Invoke-Private -File $script:adb -Arguments @('forward', '--list') -Label $Label
  @($result.Lines | ForEach-Object {
      $parts = $_ -split '\s+'
      if ($parts.Count -ge 3 -and $parts[0] -eq $script:baselineEndpoint) {
        [pscustomobject]@{ local = $parts[1]; remote = $parts[2] }
      }
    })
}

function Get-PackageState {
  param([string]$Package, [string]$Label, [switch]$BackupApks)
  $pathsResult = Invoke-Adb -Arguments @('shell', 'pm', 'path', $Package) -Label "$Label-path" -AllowFailure
  $paths = @($pathsResult.Lines | ForEach-Object {
      if ($_ -match '^package:(.+)$') { $Matches[1].Trim() }
    })
  if ($paths.Count -eq 0) {
    return [pscustomobject]@{ installed = $false; package = $Package; versionCode = $null; versionName = $null; debuggable = $null; signer = $null; backups = @() }
  }
  $dump = Invoke-Adb -Arguments @('shell', 'dumpsys', 'package', $Package) -Label "$Label-dump"
  $versionCode = ([regex]::Match($dump.Text, '(?m)^\s*versionCode=(\d+)')).Groups[1].Value
  $versionName = ([regex]::Match($dump.Text, '(?m)^\s*versionName=([^\r\n]+)')).Groups[1].Value.Trim()
  $debuggable = $dump.Text -match '(?m)^\s*(?:pkgFlags|flags)=\[[^\]]*\bDEBUGGABLE\b'
  $backups = @()
  $signer = $null
  if ($BackupApks) {
    $packageRoot = Join-Path $privateRoot ("package-backup-" + $Package.Replace('.', '-'))
    New-Item -ItemType Directory -Force -Path $packageRoot | Out-Null
    for ($i = 0; $i -lt $paths.Count; $i++) {
      $local = Join-Path $packageRoot ("$i.apk")
      Invoke-Adb -Arguments @('pull', $paths[$i], $local) -Label "$Label-pull-$i" | Out-Null
      $backups += $local
    }
    if ($backups.Count -gt 0) { $signer = Get-SignerDigest -Apk $backups[0] -Label "$Label-signer" }
  }
  [pscustomobject]@{
    installed = $true
    package = $Package
    versionCode = $versionCode
    versionName = $versionName
    debuggable = $debuggable
    signer = $signer
    backups = $backups
  }
}

function Add-PhaseResult {
  param([string]$Name, [bool]$Passed, [long]$Milliseconds, [object]$Detail)
  $script:phaseResults.Add([pscustomobject]@{ name = $Name; passed = $Passed; ms = $Milliseconds; detail = $Detail })
}

function Run-Phase {
  param([string]$Name, [scriptblock]$Body)
  $script:phase = $Name
  $watch = [Diagnostics.Stopwatch]::StartNew()
  try {
    $detail = & $Body
    $watch.Stop()
    Add-PhaseResult -Name $Name -Passed $true -Milliseconds $watch.ElapsedMilliseconds -Detail $detail
    $detail
  } catch {
    $watch.Stop()
    $message = "$Name failed: $($_.Exception.Message)"
    $script:failures.Add($message)
    Add-PrivateLog -Label "$Name-error" -Text ($_ | Out-String)
    Add-PhaseResult -Name $Name -Passed $false -Milliseconds $watch.ElapsedMilliseconds -Detail $message
    throw
  }
}

function Install-HelperRelease {
  param([string]$Reason)
  Assert-ExactIdentity -Reason $Reason | Out-Null
  $script:mutationStarted = $true
  Invoke-Au -Arguments @('--timeout', '120000', 'app', 'install', $HelperApk) -Label "$Reason-install-helper" | Out-Null
}

function Remove-TestPackage {
  param([string]$Reason)
  Assert-ExactIdentity -Reason $Reason | Out-Null
  $present = Get-PackageState -Package $testPackage -Label "$Reason-test-state"
  if ($present.installed) {
    Invoke-Adb -Arguments @('uninstall', $testPackage) -Label "$Reason-uninstall-test" -AllowFailure | Out-Null
  }
}

function Remove-NewAuForwards {
  param([object[]]$Before)
  Assert-ExactIdentity -Reason 'cleanup-forwards' | Out-Null
  $after = @(Get-ForwardState -Label 'cleanup-forwards-current')
  foreach ($entry in $after) {
    $existed = @($Before | Where-Object { $_.local -eq $entry.local -and $_.remote -eq $entry.remote }).Count -gt 0
    $owned = $entry.remote -match '^localabstract:codex_au_bridge(?:_bootstrap)?$'
    if (-not $existed -and $owned) {
      Invoke-Private -File $script:adb -Arguments @('-s', $script:baselineEndpoint, 'forward', '--remove', $entry.local) -Label 'cleanup-forward-remove' -AllowFailure | Out-Null
    }
  }
}

if (-not (Test-Path -LiteralPath $configPath -PathType Leaf)) { throw 'AU config is missing; no device action was taken' }
$config = Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json
$script:enrolledSerial = [string]$config.hardware_serial
if (-not $script:enrolledSerial) { throw 'AU is not enrolled; no device action was taken' }
$script:identityFingerprint = Get-IdentityFingerprint -Value $script:enrolledSerial
$script:au = Resolve-Au
$script:adb = Resolve-Adb -Config $config
$script:apkSigner = Resolve-ApkSigner

$artifacts = @(
  Assert-Artifact -Name 'helper' -Path $HelperApk -Expected $ExpectedHelperSha256
  Assert-Artifact -Name 'instrumentation' -Path $TestApk -Expected $ExpectedTestSha256
  Assert-Artifact -Name 'fixture' -Path $FixtureApk -Expected $ExpectedFixtureSha256
)
$releaseSigner = Get-SignerDigest -Apk $HelperApk -Label 'host-helper-signer'
$fixtureSigner = Get-SignerDigest -Apk $FixtureApk -Label 'host-fixture-signer'
if (-not (Test-Path -LiteralPath $InstrumentationHelperApk -PathType Leaf)) {
  throw 'instrumentation target APK is missing; build the signed debug target before running the device gate'
}
$instrumentationSigner = Get-SignerDigest -Apk $InstrumentationHelperApk -Label 'host-instrumentation-target-signer'
if ($instrumentationSigner -ne $releaseSigner) {
  throw 'instrumentation target signer does not match the release signer'
}

Assert-ExactIdentity -Reason 'preflight' | Out-Null
$api = (Invoke-Adb -Arguments @('shell', 'getprop', 'ro.build.version.sdk') -Label 'preflight-api').Text.Trim()
$release = (Invoke-Adb -Arguments @('shell', 'getprop', 'ro.build.version.release') -Label 'preflight-release').Text.Trim()
$preAccessibility = Get-AccessibilityState
$script:preForeground = Get-ForegroundComponent -Label 'preflight-foreground'
$preForwards = @(Get-ForwardState -Label 'preflight-forwards')
$preHelper = Get-PackageState -Package $helperPackage -Label 'preflight-helper' -BackupApks
$preFixture = Get-PackageState -Package $fixturePackage -Label 'preflight-fixture' -BackupApks
$preTest = Get-PackageState -Package $testPackage -Label 'preflight-test'

$preflight = [pscustomobject]@{
  identity = "sha256:$($script:identityFingerprint)"
  online_transports = 1
  transport = 'exact-enrolled'
  android_identity = 'matched'
  api = [int]$api
  android = $release
  helper = [pscustomobject]@{
    installed = $preHelper.installed
    versionCode = $preHelper.versionCode
    versionName = $preHelper.versionName
    debuggable = $preHelper.debuggable
    signer_matches_release = ($preHelper.signer -and $preHelper.signer -eq $releaseSigner)
  }
  accessibility_au_enabled = ($preAccessibility.entries -like "*$accessibilityComponent*" -or $preAccessibility.entries -like "*$accessibilityShortComponent*")
  accessibility_entry_count = if ($preAccessibility.entries -and $preAccessibility.entries -ne 'null') { @($preAccessibility.entries.Split(':') | Where-Object { $_ }).Count } else { 0 }
  forward_count = $preForwards.Count
  fixture_installed = $preFixture.installed
  fixture_versionCode = $preFixture.versionCode
  test_package_installed = $preTest.installed
}

if (-not $Execute) {
  $summary = [pscustomobject]@{
    schema = 1
    mode = 'preflight-only'
    raw_log = $rawLog
    artifacts = $artifacts
    preflight = $preflight
    device_changed = $false
  }
  $json = $summary | ConvertTo-Json -Depth 8
  Set-Content -LiteralPath $privateSummary -Encoding UTF8 -Value $json
  $summary | ConvertTo-Json -Depth 8 -Compress
  return
}

$instrumentation = $null
$runtime = $null
$fgs = $null
$fixture = $null
$post = $null

try {
  $instrumentation = Run-Phase -Name 'instrumentation' -Body {
    # AndroidX Test loads Kotlin runtime classes in the target process. The
    # production release intentionally lets R8 remove unused test-only runtime
    # code, so run instrumentation against the persistent-signed debug target;
    # the release is installed and tested in the following runtime phases.
    Assert-ExactIdentity -Reason 'instrumentation-target' | Out-Null
    $script:mutationStarted = $true
    Invoke-Adb -Arguments @('install', '-r', $InstrumentationHelperApk) -Label 'instrumentation-target-install-helper' | Out-Null
    $instrumentationTarget = Get-PackageState -Package $helperPackage -Label 'instrumentation-target-state'
    if (-not $instrumentationTarget.installed -or -not $instrumentationTarget.debuggable) {
      throw 'instrumentation target was not the signed debuggable test variant'
    }
    Assert-ExactIdentity -Reason 'instrumentation-test-install' | Out-Null
    Invoke-Adb -Arguments @('install', '-r', '-t', $TestApk) -Label 'instrumentation-install-test' | Out-Null
    Assert-ExactIdentity -Reason 'instrumentation-run' | Out-Null
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $run = Invoke-Adb -Arguments @('shell', 'timeout', '240', 'am', 'instrument', '-w', '-r', "$testPackage/androidx.test.runner.AndroidJUnitRunner") -Label 'instrumentation-run'
    $watch.Stop()
    $countMatch = [regex]::Match($run.Text, 'OK \((\d+) tests?\)')
    if (-not $countMatch.Success) {
      $allCounts = [regex]::Matches($run.Text, 'numtests=(\d+)')
      $testCount = if ($allCounts.Count) { [int]$allCounts[$allCounts.Count - 1].Groups[1].Value } else { 0 }
    } else {
      $testCount = [int]$countMatch.Groups[1].Value
    }
    if ($run.Text -notmatch 'INSTRUMENTATION_CODE:\s*-1|OK \(\d+ tests?\)') { throw 'instrumentation did not report success' }
    Remove-TestPackage -Reason 'instrumentation-cleanup'
    [pscustomobject]@{
      target = 'signed-debug'
      target_debuggable = $instrumentationTarget.debuggable
      target_signer_matches_release = $true
      tests = $testCount
      ms = $watch.ElapsedMilliseconds
      test_package_removed = $true
    }
  }

  $runtime = Run-Phase -Name 'release-runtime' -Body {
    Install-HelperRelease -Reason 'release-restore'
    Set-TestAccessibility
    Assert-ExactIdentity -Reason 'release-capabilities' | Out-Null
    $cap = Invoke-Au -Arguments @('--timeout', '30000', '-j', 'cap') -Label 'release-capabilities'
    $capJson = $cap.Text | ConvertFrom-Json
    $runAs = Invoke-Adb -Arguments @('shell', 'run-as', $helperPackage, 'id') -Label 'release-run-as-rejection' -AllowFailure
    if ($runAs.ExitCode -eq 0) { throw 'run-as unexpectedly succeeded for the release helper' }
    $serviceDump = Invoke-Adb -Arguments @('shell', 'dumpsys', 'activity', 'services', $helperPackage) -Label 'release-service-dump'
    $notificationDump = Invoke-Adb -Arguments @('shell', 'dumpsys', 'notification') -Label 'release-notification-dump'
    $packageDump = Invoke-Adb -Arguments @('shell', 'dumpsys', 'package', $helperPackage) -Label 'release-package-dump'
    $snapshot = Invoke-Au -Arguments @('--timeout', '30000', '-w', 'ui', 'snap', '--compact', '--frontier') -Label 'release-semantic-snapshot'
    $choice = Invoke-Au -Arguments @('--timeout', '15000', '-w', 'ui', 'find', 'clickable=true#0') -Label 'release-semantic-choice' -AllowFailure
    $query = Invoke-Au -Arguments @('--timeout', '15000', '-w', 'ui', 'find', 'role=text#0') -Label 'release-semantic-query' -AllowFailure
    $forwards = @(Get-ForwardState -Label 'release-forwards-after-auth')
    $bootstrapForward = @($forwards | Where-Object { $_.remote -eq 'localabstract:codex_au_bridge_bootstrap' }).Count -gt 0
    if ($bootstrapForward) { throw 'temporary bootstrap forward remained after authentication' }
    $postHelper = Get-PackageState -Package $helperPackage -Label 'release-helper-state' -BackupApks
    if (-not $postHelper.installed -or $postHelper.debuggable -or $postHelper.signer -ne $releaseSigner) { throw 'release helper state did not match the signed non-debuggable artifact' }
    [pscustomobject]@{
      helper_versionCode = $postHelper.versionCode
      helper_versionName = $postHelper.versionName
      protocol = if ($capJson.PSObject.Properties.Name -contains 'protocol') { $capJson.protocol } elseif ($capJson.PSObject.Properties.Name -contains 'helper') { $capJson.helper.protocol } else { $null }
      run_as_rejected = $true
      foreground_service = ($serviceDump.Text -match 'AuBridgeService')
      notification_present = ($notificationDump.Text -match [regex]::Escape($helperPackage))
      dump_protected = ($packageDump.Text -match 'AuBridgeService' -and $packageDump.Text -match 'android\.permission\.DUMP')
      snapshot_ok = ($snapshot.Text -match '"o"\s*:\s*1')
      choices_ok = ($choice.ExitCode -eq 0 -and $choice.Text -match '"o"\s*:\s*1')
      query_ok = ($query.ExitCode -eq 0 -and $query.Text -match '"o"\s*:\s*1')
      bootstrap_authenticated = $true
      bootstrap_forward_removed = $true
    }
  }

  $fgs = Run-Phase -Name 'api33-fgs-recovery' -Body {
    if ([int]$api -ne 33) { throw "expected API 33 tablet, observed API $api" }
    $permissionDump = Invoke-Adb -Arguments @('shell', 'dumpsys', 'package', $helperPackage) -Label 'fgs-permissions-before'
    $cameraGranted = $permissionDump.Text -match 'android\.permission\.CAMERA:\s+granted=true'
    $microphoneGranted = $permissionDump.Text -match 'android\.permission\.RECORD_AUDIO:\s+granted=true'
    $locationGranted = $permissionDump.Text -match 'android\.permission\.ACCESS_(?:FINE|COARSE)_LOCATION:\s+granted=true'
    if ($cameraGranted -or $microphoneGranted -or $locationGranted) { throw 'sensitive runtime permission was already granted; no permission was changed' }
    Assert-ExactIdentity -Reason 'fgs-force-stop-helper' | Out-Null
    Invoke-Adb -Arguments @('shell', 'am', 'force-stop', '--user', '0', $helperPackage) -Label 'fgs-force-stop-helper' | Out-Null
    Set-TestAccessibility
    # Android marks a force-stopped package's process as bad until it is
    # explicitly launched. Start the harmless operator activity to clear that
    # state, then verify the authenticated service path.
    Assert-ExactIdentity -Reason 'fgs-recovery-launch' | Out-Null
    Invoke-Adb -Arguments @('shell', 'am', 'start', '-W', '-n', "$helperPackage/.MainActivity") -Label 'fgs-recovery-launch' | Out-Null
    $watch = [Diagnostics.Stopwatch]::StartNew()
    Assert-ExactIdentity -Reason 'fgs-recovery-capabilities' | Out-Null
    Invoke-Au -Arguments @('--timeout', '30000', '-j', 'cap') -Label 'fgs-recovery-capabilities' | Out-Null
    $snap = Invoke-Au -Arguments @('--timeout', '30000', '-w', 'ui', 'snap', '--compact', '--frontier') -Label 'fgs-recovery-snapshot'
    $watch.Stop()
    $forwards = @(Get-ForwardState -Label 'fgs-recovery-forwards')
    if (@($forwards | Where-Object { $_.remote -eq 'localabstract:codex_au_bridge_bootstrap' }).Count -gt 0) { throw 'bootstrap forward remained after recovery' }
    [pscustomobject]@{
      api = [int]$api
      camera_granted = $false
      microphone_granted = $false
      location_granted = $false
      recovered = ($snap.Text -match '"o"\s*:\s*1')
      recovery_ms = $watch.ElapsedMilliseconds
      listeners_authenticated = $true
    }
  }

  $fixture = Run-Phase -Name 'fixture-lifecycle' -Body {
    Assert-ExactIdentity -Reason 'fixture-install' | Out-Null
    Invoke-Au -Arguments @('--timeout', '120000', 'app', 'install', $FixtureApk) -Label 'fixture-install' | Out-Null
    Assert-ExactIdentity -Reason 'fixture-launch' | Out-Null
    Invoke-Au -Arguments @('--timeout', '30000', 'app', 'start', $fixturePackage) -Label 'fixture-launch' | Out-Null
    Invoke-AuSnapshotRetry -Label 'fixture-snapshot-before' | Out-Null
    $batch = Invoke-Au -Arguments @('--timeout', '30000', '-w', 'b', "ui set 'role=input#0' 'AU gate'; ui tap 'role=switch#0'") -Label 'fixture-semantic-batch'
    Assert-ExactIdentity -Reason 'fixture-force-stop' | Out-Null
    Invoke-Au -Arguments @('--timeout', '30000', 'app', 'stop', $fixturePackage) -Label 'fixture-force-stop' | Out-Null
    Assert-ExactIdentity -Reason 'fixture-relaunch' | Out-Null
    Invoke-Au -Arguments @('--timeout', '30000', 'app', 'start', $fixturePackage) -Label 'fixture-relaunch' | Out-Null
    $after = Invoke-AuSnapshotRetry -Label 'fixture-snapshot-after'
    [pscustomobject]@{
      installed = $true
      launched = $true
      batch_ok = ($batch.Text -match '"o"\s*:\s*1')
      recovered_after_force_stop = ($after.Text -match '"o"\s*:\s*1')
      preexisting = $preFixture.installed
    }
  }
} catch {
  $script:failures.Add("execution halted after $($script:phase); cleanup continued")
} finally {
  try { Remove-TestPackage -Reason 'final-cleanup-test' } catch { $script:cleanupNotes.Add("test cleanup failed: $($_.Exception.Message)") }
  try {
    Assert-ExactIdentity -Reason 'final-cleanup-fixture' | Out-Null
    $currentFixture = Get-PackageState -Package $fixturePackage -Label 'final-fixture-state'
    if ($preFixture.installed) {
      if ($preFixture.backups.Count -eq 1) {
        Invoke-Adb -Arguments @('install', '-r', '-d', $preFixture.backups[0]) -Label 'restore-preexisting-fixture' | Out-Null
      } elseif ($preFixture.backups.Count -gt 1) {
        Invoke-Adb -Arguments (@('install-multiple', '-r', '-d') + $preFixture.backups) -Label 'restore-preexisting-fixture-splits' | Out-Null
      }
    } elseif ($currentFixture.installed) {
      Invoke-Adb -Arguments @('uninstall', $fixturePackage) -Label 'remove-new-fixture' | Out-Null
    }
  } catch { $script:cleanupNotes.Add("fixture cleanup failed: $($_.Exception.Message)") }
  try { Install-HelperRelease -Reason 'final-helper-release' } catch { $script:cleanupNotes.Add("helper restore failed: $($_.Exception.Message)") }
  try { Restore-Accessibility -State $preAccessibility } catch { $script:cleanupNotes.Add("accessibility restore failed: $($_.Exception.Message)") }
  try { Remove-NewAuForwards -Before $preForwards } catch { $script:cleanupNotes.Add("forward cleanup failed: $($_.Exception.Message)") }
  try { Restore-ForegroundComponent } catch { $script:cleanupNotes.Add("foreground restore failed: $($_.Exception.Message)") }
}

try {
  Assert-ExactIdentity -Reason 'postflight' | Out-Null
  $postAccessibility = Get-AccessibilityState
  $postForwards = @(Get-ForwardState -Label 'postflight-forwards')
  $postHelper = Get-PackageState -Package $helperPackage -Label 'postflight-helper' -BackupApks
  $postFixture = Get-PackageState -Package $fixturePackage -Label 'postflight-fixture'
  $postTest = Get-PackageState -Package $testPackage -Label 'postflight-test'
  $newForwards = @($postForwards | Where-Object {
      $candidate = $_
      @($preForwards | Where-Object { $_.local -eq $candidate.local -and $_.remote -eq $candidate.remote }).Count -eq 0
    })
  $post = [pscustomobject]@{
    identity_matched = $true
    helper_release_installed = ($postHelper.installed -and -not $postHelper.debuggable -and $postHelper.signer -eq $releaseSigner)
    helper_signer_continuity = ($postHelper.signer -eq $releaseSigner)
    accessibility_restored_exactly = ($postAccessibility.entries -eq $preAccessibility.entries -and $postAccessibility.enabled -eq $preAccessibility.enabled)
    fixture_restored = if ($preFixture.installed) { $postFixture.installed -and $postFixture.versionCode -eq $preFixture.versionCode } else { -not $postFixture.installed }
    test_package_absent = (-not $postTest.installed)
    new_forward_count = $newForwards.Count
    cleanup_complete = ($script:cleanupNotes.Count -eq 0 -and $newForwards.Count -eq 0 -and -not $postTest.installed)
    intentional_differences = @(
      if (-not $preHelper.installed -or $preHelper.debuggable -or $preHelper.signer -ne $releaseSigner) { 'helper is now the exact signed non-debuggable release' }
    )
  }
} catch {
  $script:failures.Add("postflight failed: $($_.Exception.Message)")
  $post = [pscustomobject]@{ identity_matched = $false; cleanup_complete = $false }
}

$summary = [pscustomobject]@{
  schema = 1
  mode = 'execute'
  raw_log = $rawLog
  artifacts = $artifacts
  preflight = $preflight
  instrumentation = $instrumentation
  runtime = $runtime
  api33_fgs = $fgs
  fixture = $fixture
  phases = $script:phaseResults
  failures = $script:failures
  cleanup_notes = $script:cleanupNotes
  postflight = $post
  passed = ($script:failures.Count -eq 0 -and $script:cleanupNotes.Count -eq 0 -and $post.cleanup_complete)
}
$json = $summary | ConvertTo-Json -Depth 10
Set-Content -LiteralPath $privateSummary -Encoding UTF8 -Value $json
$summary | ConvertTo-Json -Depth 10 -Compress
if (-not $summary.passed) { exit 1 }
