param(
  [string]$Apk = (Join-Path $PSScriptRoot '..\android\aubridge\app\build\outputs\apk\release\app-release.apk'),
  [string]$SdkRoot = (Join-Path $env:LOCALAPPDATA 'Android\Sdk'),
  [string]$ExpectedSignerSha256,
  [ValidateSet('Helper', 'Fixture')]
  [string]$Profile = 'Helper',
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

function Get-XmlTreeElementBlocks {
  param(
    [Parameter(Mandatory = $true)][string]$Text,
    [Parameter(Mandatory = $true)][string]$Element
  )

  $lines = $Text -split "`r?`n"
  $blocks = [System.Collections.Generic.List[string]]::new()
  $pattern = '^(?<indent>\s*)E:\s+' + [regex]::Escape($Element) + '(?:\s|\()'
  for ($index = 0; $index -lt $lines.Count; $index++) {
    $start = [regex]::Match($lines[$index], $pattern)
    if (-not $start.Success) {
      continue
    }
    $indent = $start.Groups['indent'].Value.Length
    $end = $index + 1
    while ($end -lt $lines.Count) {
      $nextElement = [regex]::Match($lines[$end], '^(?<indent>\s*)E:\s+')
      if ($nextElement.Success -and $nextElement.Groups['indent'].Value.Length -le $indent) {
        break
      }
      $end++
    }
    $blocks.Add(($lines[$index..($end - 1)] -join [Environment]::NewLine))
    $index = $end - 1
  }
  return $blocks
}

function Get-CompiledUInt32Attribute {
  param(
    [Parameter(Mandatory = $true)][string]$Block,
    [Parameter(Mandatory = $true)][string]$Attribute
  )

  $pattern = '(?im)^\s*A:\s+.*android:' + [regex]::Escape($Attribute) +
    '(?:\([^)]*\))?\s*=\s*(?:\(type\s+[^)]*\)\s*)?(?<value>-?(?:0[xX][0-9a-fA-F]+|\d+))'
  $match = [regex]::Match($Block, $pattern)
  if (-not $match.Success) {
    throw "Compiled AndroidManifest.xml attribute was not found: android:$Attribute"
  }

  $raw = $match.Groups['value'].Value
  if ($raw -match '^0[xX]') {
    $hex = $raw.Substring(2)
    if ($hex.Length -gt 8) {
      $prefix = $hex.Substring(0, $hex.Length - 8)
      if ($prefix -notmatch '^[fF]+$') {
        throw "Compiled android:$Attribute value is outside uint32: $raw"
      }
      $hex = $hex.Substring($hex.Length - 8)
    }
    $value = [Convert]::ToUInt64($hex, 16)
  } else {
    $value = [long]::Parse(
      $raw,
      [Globalization.NumberStyles]::Integer,
      [Globalization.CultureInfo]::InvariantCulture)
    if ($value -lt 0) {
      $value += 4294967296L
    }
  }
  if ($value -lt 0 -or $value -gt 4294967295L) {
    throw "Compiled android:$Attribute value is outside uint32: $raw"
  }
  return [uint64]$value
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

$expectedPackage = if ($Profile -eq 'Fixture') { 'dev.codex.aubench' } else { 'dev.codex.aubridge' }
$module = if ($Profile -eq 'Fixture') { 'fixture' } else { 'app' }
Add-Check 'package' ($badging -match ("package: name='" + [regex]::Escape($expectedPackage) + "'")) $expectedPackage
Add-Check 'min-sdk' ($badging -match "sdkVersion:'26'") '26'
Add-Check 'target-sdk' ($badging -match "targetSdkVersion:'36'") '36'
Add-Check 'compile-sdk-source' ((Get-Content -Raw (Join-Path $PSScriptRoot "..\android\aubridge\$module\build.gradle")) -match 'compileSdk\s*=\s*36') 'build.gradle compileSdk 36'

if ($Profile -eq 'Helper') {
$serviceBlocks = @(Get-XmlTreeElementBlocks -Text $manifest -Element 'service')
$bridgeServiceBlocks = @($serviceBlocks | Where-Object {
    $_ -match 'android:name(?:\([^)]*\))?="dev\.codex\.aubridge\.AuBridgeService"(?:\s|$)'
  })
Add-Check 'bridge-service-count' ($bridgeServiceBlocks.Count -eq 1) "exact AuBridgeService blocks: $($bridgeServiceBlocks.Count)"
$bridgeService = $bridgeServiceBlocks[0]

foreach ($permission in @(
    'android.permission.CAMERA',
    'android.permission.RECORD_AUDIO',
    'android.permission.POST_NOTIFICATIONS',
    'android.permission.FOREGROUND_SERVICE',
    'android.permission.FOREGROUND_SERVICE_CAMERA',
    'android.permission.FOREGROUND_SERVICE_MICROPHONE',
    'android.permission.FOREGROUND_SERVICE_SPECIAL_USE',
    'android.permission.WAKE_LOCK'
  )) {
  Add-Check "permission:$permission" ($manifest -match [regex]::Escape(('"' + $permission + '"'))) $permission
}
foreach ($permission in @(
    'android.permission.ACCESS_FINE_LOCATION',
    'android.permission.ACCESS_COARSE_LOCATION',
    'android.permission.FOREGROUND_SERVICE_LOCATION'
  )) {
  Add-Check "permission-absent:$permission" (-not ($manifest -match [regex]::Escape(('"' + $permission + '"')))) $permission
}
Add-Check 'no-internet-permission' (-not ($manifest -match 'android\.permission\.INTERNET')) 'absent'
  Add-Check 'release-non-debuggable' (-not ($badging -match 'application-debuggable')) 'application-debuggable absent'

  Add-Check 'test-components-absent' (-not ($manifest -match 'dev\.codex\.aubridge\.(TestActivity|TestNotificationReceiver)')) 'debug-only test components absent from release'
  $applicationBlocks = @(Get-XmlTreeElementBlocks -Text $manifest -Element 'application')
  Add-Check 'application-count' ($applicationBlocks.Count -eq 1) "exact application blocks: $($applicationBlocks.Count)"
  $application = $applicationBlocks[0]
  $androidNs = '(?:android|http://schemas\.android\.com/apk/res/android)'
  Add-Check 'allow-backup-disabled' ($application -match ($androidNs + ':allowBackup(?:\([^)]*\))?=\s*(?:(?:\(type\s+0x12\)\s*)?0x0|false)')) 'android:allowBackup=false'
  Add-Check 'cleartext-disabled' ($application -match ($androidNs + ':usesCleartextTraffic(?:\([^)]*\))?=\s*(?:(?:\(type\s+0x12\)\s*)?0x0|false)')) 'android:usesCleartextTraffic=false'
  $receiverBlocks = @(Get-XmlTreeElementBlocks -Text $manifest -Element 'receiver')
  $providerBlocks = @(Get-XmlTreeElementBlocks -Text $manifest -Element 'provider')
  Add-Check 'no-production-receivers' ($receiverBlocks.Count -eq 0) "unexpected production receiver blocks: $($receiverBlocks.Count)"
  Add-Check 'no-production-providers' ($providerBlocks.Count -eq 0) "unexpected production provider blocks: $($providerBlocks.Count)"
  Add-Check 'launcher-activity' ($manifest -match '(?s)dev\.codex\.aubridge\.MainActivity.*?:exported\(.*?\)=true.*?android\.intent\.action\.MAIN.*?android\.intent\.category\.LAUNCHER') 'exported MAIN/LAUNCHER'
  $bridgeDeclarationOk =
    ($bridgeService -match 'android:permission(?:\([^)]*\))?="android\.permission\.DUMP"') -and
    ($bridgeService -match 'android:exported(?:\([^)]*\))?=true') -and
    ($bridgeService -match 'dev\.codex\.aubridge\.action\.BRIDGE')
  Add-Check 'bridge-service' $bridgeDeclarationOk 'exact component exported only to privileged adb shell callers with DUMP'

  $bridgeFgsMask = Get-CompiledUInt32Attribute -Block $bridgeService -Attribute 'foregroundServiceType'
  $expectedBridgeFgsMask = [uint64]0x400000c0
  $dataSyncFgsMask = [uint64]0x00000001
  $bridgeFgsMaskHex = '0x{0:x8}' -f $bridgeFgsMask
  Add-Check 'bridge-fgs-types' ($bridgeFgsMask -eq $expectedBridgeFgsMask) "$bridgeFgsMaskHex expected 0x400000c0"
  Add-Check 'bridge-no-data-sync-type' (($bridgeFgsMask -band $dataSyncFgsMask) -eq 0) "$bridgeFgsMaskHex dataSync bit clear"
  Add-Check 'bridge-special-use-subtype' ($bridgeService -match 'android\.app\.PROPERTY_SPECIAL_USE_FGS_SUBTYPE') 'special-use subtype property on exact bridge service'
Add-Check 'accessibility-service' ($manifest -match '(?s)dev\.codex\.aubridge\.AubridgeAccessibilityService.*?android\.permission\.BIND_ACCESSIBILITY_SERVICE.*?android\.accessibilityservice\.AccessibilityService') 'accessibility declaration'
  Add-Check 'notification-service' ($manifest -match '(?s)dev\.codex\.aubridge\.AubridgeNotificationListener.*?android\.permission\.BIND_NOTIFICATION_LISTENER_SERVICE.*?android\.service\.notification\.NotificationListenerService') 'notification declaration'
} else {
  Add-Check 'no-declared-permissions' (-not ($manifest -match 'uses-permission')) 'fixture declares no permissions'
  Add-Check 'fixture-launcher' ($manifest -match '(?s)dev\.codex\.aubench\.MainActivity.*?:exported\(.*?\)=true.*?android\.intent\.action\.MAIN.*?android\.intent\.category\.LAUNCHER') 'exported MAIN/LAUNCHER'
  Add-Check 'no-components-beyond-activity' (-not ($manifest -match '(?m)^\s*E: (service|receiver|provider)')) 'no services, receivers, or providers'
}

Add-Check 'apk-signed' ($signing -match 'Verified using v1 scheme|Verified using v2 scheme|Verified using v3 scheme|Verified using v4 scheme') 'apksigner verification passed'
$signerMatch = [regex]::Match($signing, 'Signer #1 certificate SHA-256 digest:\s*([0-9a-fA-F]+)')
if (-not $signerMatch.Success) {
  throw 'APK validation failed: signer digest was not reported by apksigner'
}
$signer = $signerMatch.Groups[1].Value.ToLowerInvariant()
if ($Profile -eq 'Helper' -and [string]::IsNullOrWhiteSpace($ExpectedSignerSha256)) {
  throw 'APK validation failed: ExpectedSignerSha256 is required for the helper profile'
}
if ($ExpectedSignerSha256) {
  Add-Check 'signer-identity' ($signer -eq $ExpectedSignerSha256.ToLowerInvariant()) $signer
}

$summary = [pscustomobject]@{
  apk = (Resolve-Path -LiteralPath $Apk).Path
  sha256 = (Get-FileHash -LiteralPath $Apk -Algorithm SHA256).Hash.ToLowerInvariant()
  bytes = (Get-Item -LiteralPath $Apk).Length
  signer_sha256 = $signer
  profile = $Profile
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
