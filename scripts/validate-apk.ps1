param(
  [string]$Apk = (Join-Path $PSScriptRoot '..\android\aubridge\app\build\outputs\apk\release\app-release.apk'),
  [string]$SdkRoot = (Join-Path $env:LOCALAPPDATA 'Android\Sdk'),
  [string]$ExpectedSignerSha256,
  [switch]$Json
)

$ErrorActionPreference = 'Stop'

function Invoke-TextTool {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [Parameter(Mandatory = $true)][string[]]$Arguments
  )

  $output = & $Path @Arguments 2>&1
  $exitCode = $LASTEXITCODE
  $text = ($output | ForEach-Object { $_.ToString() }) -join [Environment]::NewLine
  if ($exitCode -ne 0) {
    throw "$(Split-Path -Leaf $Path) failed ($exitCode): $text"
  }
  return $text
}

function Add-Check {
  param(
    [Parameter(Mandatory = $true)][string]$Name,
    [Parameter(Mandatory = $true)][bool]$Passed,
    [Parameter(Mandatory = $true)][string]$Detail
  )

  $script:results.Add([pscustomobject]@{
      name = $Name
      pass = $Passed
      detail = $Detail
    })
  if (-not $Passed) {
    throw "APK validation failed: $Name - $Detail"
  }
}

if (-not (Test-Path -LiteralPath $Apk -PathType Leaf)) {
  throw "APK not found: $Apk"
}

$aapt2 = Join-Path $SdkRoot 'build-tools\36.0.0\aapt2.exe'
$apksigner = Join-Path $SdkRoot 'build-tools\36.0.0\apksigner.bat'
if (-not (Test-Path -LiteralPath $aapt2 -PathType Leaf)) {
  throw "aapt2 not found: $aapt2"
}
if (-not (Test-Path -LiteralPath $apksigner -PathType Leaf)) {
  throw "apksigner not found: $apksigner"
}

$script:results = [System.Collections.Generic.List[object]]::new()
$badging = Invoke-TextTool -Path $aapt2 -Arguments @('dump', 'badging', $Apk)
$manifest = Invoke-TextTool -Path $aapt2 -Arguments @('dump', 'xmltree', '--file', 'AndroidManifest.xml', $Apk)
$signing = Invoke-TextTool -Path $apksigner -Arguments @('verify', '--verbose', '--print-certs', $Apk)

Add-Check 'package' ($badging -match "package: name='dev\.codex\.aubridge'") 'dev.codex.aubridge'
Add-Check 'min-sdk' ($badging -match "sdkVersion:'30'") '30'
Add-Check 'target-sdk' ($badging -match "targetSdkVersion:'36'") '36'
Add-Check 'compile-sdk-source' ((Get-Content -Raw (Join-Path $PSScriptRoot '..\android\aubridge\app\build.gradle')) -match 'compileSdk\s*=\s*36') 'build.gradle compileSdk 36'

foreach ($permission in @(
    'android.permission.CAMERA',
    'android.permission.RECORD_AUDIO',
    'android.permission.ACCESS_FINE_LOCATION',
    'android.permission.ACCESS_COARSE_LOCATION',
    'android.permission.POST_NOTIFICATIONS',
    'android.permission.FOREGROUND_SERVICE',
    'android.permission.FOREGROUND_SERVICE_CAMERA',
    'android.permission.FOREGROUND_SERVICE_MICROPHONE',
    'android.permission.FOREGROUND_SERVICE_LOCATION',
    'android.permission.WAKE_LOCK'
  )) {
  Add-Check "permission:$permission" ($manifest -match [regex]::Escape(('"' + $permission + '"'))) $permission
}
Add-Check 'no-internet-permission' (-not ($manifest -match 'android\.permission\.INTERNET')) 'absent'

Add-Check 'test-activity' ($manifest -match '(?s)dev\.codex\.aubridge\.TestActivity.*?android\.permission\.DUMP.*?:exported\(.*?\)=true') 'exported with DUMP permission'
Add-Check 'launcher-activity' ($manifest -match '(?s)dev\.codex\.aubridge\.MainActivity.*?:exported\(.*?\)=true.*?android\.intent\.action\.MAIN.*?android\.intent\.category\.LAUNCHER') 'exported MAIN/LAUNCHER'
Add-Check 'bridge-service' ($manifest -match '(?s)dev\.codex\.aubridge\.AuBridgeService.*?android\.permission\.FOREGROUND_SERVICE.*?:exported\(.*?\)=true.*?dev\.codex\.aubridge\.action\.BRIDGE') 'exported authenticated bridge bootstrap'
Add-Check 'accessibility-service' ($manifest -match '(?s)dev\.codex\.aubridge\.AubridgeAccessibilityService.*?android\.permission\.BIND_ACCESSIBILITY_SERVICE.*?android\.accessibilityservice\.AccessibilityService') 'accessibility declaration'
Add-Check 'notification-service' ($manifest -match '(?s)dev\.codex\.aubridge\.AubridgeNotificationListener.*?android\.permission\.BIND_NOTIFICATION_LISTENER_SERVICE.*?android\.service\.notification\.NotificationListenerService') 'notification declaration'

Add-Check 'apk-signed' ($signing -match 'Verified using v1 scheme|Verified using v2 scheme|Verified using v3 scheme|Verified using v4 scheme') 'apksigner verification passed'
$signerMatch = [regex]::Match($signing, 'Signer #1 certificate SHA-256 digest:\s*([0-9a-fA-F]+)')
if (-not $signerMatch.Success) {
  throw 'APK validation failed: signer digest was not reported by apksigner'
}
$signer = $signerMatch.Groups[1].Value.ToLowerInvariant()
if ($ExpectedSignerSha256) {
  Add-Check 'signer-identity' ($signer -eq $ExpectedSignerSha256.ToLowerInvariant()) $signer
}

$summary = [pscustomobject]@{
  apk = (Resolve-Path -LiteralPath $Apk).Path
  sha256 = (Get-FileHash -LiteralPath $Apk -Algorithm SHA256).Hash.ToLowerInvariant()
  bytes = (Get-Item -LiteralPath $Apk).Length
  signer_sha256 = $signer
  checks = $results
}
if ($Json) {
  $summary | ConvertTo-Json -Depth 4 -Compress
} else {
  foreach ($result in $results) {
    "{0} {1} {2}" -f ($(if ($result.pass) { 'PASS' } else { 'FAIL' }), $result.name, $result.detail)
  }
  "PASS apk $($summary.bytes) bytes sha256 $($summary.sha256) signer $($summary.signer_sha256)"
}
