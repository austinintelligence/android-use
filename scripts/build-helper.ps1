param(
  [switch]$Release,
  [string]$SdkRoot = $(if ($env:ANDROID_SDK_ROOT) { $env:ANDROID_SDK_ROOT } elseif ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' })
)

$ErrorActionPreference = 'Stop'
$projectRoot = Join-Path $PSScriptRoot 'helper'
$stateRoot = Join-Path $env:LOCALAPPDATA 'Codex\android-use'
$toolRoot = Join-Path $stateRoot 'tools'
$gradleRoot = Join-Path $toolRoot 'gradle-9.1.0'
$gradle = Join-Path $gradleRoot 'bin\gradle.bat'
$gradleSha256 = 'a17ddd85a26b6a7f5ddb71ff8b05fc5104c0202c6e64782429790c933686c806'
$keyRoot = Join-Path $stateRoot 'keys'
$keystore = Join-Path $keyRoot 'aubridge.keystore'
$signing = Join-Path $stateRoot 'signing.properties'

function Resolve-JavaHome {
  $embeddedJava = Get-ChildItem (Join-Path $toolRoot 'temurin17') -Directory -ErrorAction SilentlyContinue |
    Where-Object { Test-Path (Join-Path $_.FullName 'bin\java.exe') } |
    Select-Object -First 1 -ExpandProperty FullName
  $candidates = @(
    $env:JAVA_HOME,
    $embeddedJava,
    (Get-ChildItem 'C:\Program Files\Eclipse Adoptium' -Directory -ErrorAction SilentlyContinue | Sort-Object Name -Descending | Select-Object -First 1 -ExpandProperty FullName),
    (Get-ChildItem 'C:\Program Files\Microsoft\jdk*' -Directory -ErrorAction SilentlyContinue | Sort-Object Name -Descending | Select-Object -First 1 -ExpandProperty FullName)
  ) | Where-Object { $_ -and (Test-Path (Join-Path $_ 'bin\java.exe')) }
  if ($candidates.Count -eq 0) { throw 'JDK 17 is required; install EclipseAdoptium.Temurin.17.JDK first.' }
  # A one-item PowerShell pipeline unwraps to a String, where `[0]` is the
  # first character rather than the first candidate. Keep it as an array.
  return @($candidates)[0]
}

New-Item -ItemType Directory -Force -Path $stateRoot,$toolRoot,$keyRoot | Out-Null
# Parenthesize the invocation.  Without this, PowerShell treats the command
# name as a bare assignment expression and Gradle can receive only the first
# character of a path (for example `C`) as JAVA_HOME.
$env:JAVA_HOME = (Resolve-JavaHome)
$env:ANDROID_HOME = $sdkRoot
$env:ANDROID_SDK_ROOT = $sdkRoot

if (!(Test-Path $gradle)) {
  $archive = Join-Path $toolRoot 'gradle-9.1.0-bin.verified.zip'
  if (!(Test-Path $archive)) {
    Invoke-WebRequest -UseBasicParsing 'https://services.gradle.org/distributions/gradle-9.1.0-bin.zip' -OutFile $archive
  }
  $actualGradleSha256 = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($actualGradleSha256 -ne $gradleSha256) {
    throw "Gradle archive SHA-256 mismatch: expected $gradleSha256, got $actualGradleSha256"
  }
  Expand-Archive -LiteralPath $archive -DestinationPath $toolRoot -Force
}

$android = Join-Path $env:LOCALAPPDATA 'Microsoft\WinGet\Links\android.exe'
$sdkManagerCandidates = @(
  (Join-Path $sdkRoot 'cmdline-tools\latest\bin\sdkmanager.bat'),
  (Join-Path $sdkRoot 'cmdline-tools\bin\sdkmanager.bat'),
  (Join-Path $sdkRoot 'tools\bin\sdkmanager.bat')
) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf }
if (!(Test-Path (Join-Path $sdkRoot 'platforms\android-36')) -or !(Test-Path (Join-Path $sdkRoot 'build-tools\36.0.0'))) {
  if (Test-Path $android) {
    & $android --no-metrics sdk install 'platforms/android-36' 'build-tools/36.0.0' 'platform-tools'
  } elseif ($sdkManagerCandidates.Count -gt 0) {
    & $sdkManagerCandidates[0] "--sdk_root=$sdkRoot" 'platforms;android-36' 'build-tools;36.0.0' 'platform-tools'
  } else {
    throw 'Android CLI or sdkmanager is required to install API 36 and Build Tools 36.0.0.'
  }
  if ($LASTEXITCODE -ne 0) { throw 'Android SDK installation failed.' }
}

if (!(Test-Path $keystore)) {
  $bytes = New-Object byte[] 24
  [System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
  $password = [Convert]::ToBase64String($bytes).Replace('+','A').Replace('/','B').Replace('=','C')
  & (Join-Path $env:JAVA_HOME 'bin\keytool.exe') -genkeypair -keystore $keystore -storepass $password -keypass $password -alias aubridge -keyalg RSA -keysize 4096 -validity 3650 -dname 'CN=AU Bridge, OU=Codex, O=Local, L=Local, ST=Local, C=US' -noprompt
  if ($LASTEXITCODE -ne 0) { throw 'Persistent helper signing key creation failed.' }
  $escapedKeystore = $keystore.Replace('\','\\')
  @(
    "storeFile=$escapedKeystore"
    "storePassword=$password"
    'keyAlias=aubridge'
    "keyPassword=$password"
  ) | Set-Content -LiteralPath $signing -Encoding Ascii
}

$task = if ($Release) { ':app:assembleRelease' } else { ':app:assembleDebug' }
Push-Location $projectRoot
try {
  & $gradle --no-daemon --stacktrace $task
  if ($LASTEXITCODE -ne 0) { throw "Gradle task $task failed." }
  $apkRelative = if ($Release) { 'app\build\outputs\apk\release\app-release.apk' } else { 'app\build\outputs\apk\debug\app-debug.apk' }
  $apk = Join-Path $projectRoot $apkRelative
  & (Join-Path $PSScriptRoot 'validate-apk.ps1') -Apk $apk -SdkRoot $sdkRoot
  if ($LASTEXITCODE -ne 0) { throw 'Packed APK validation failed.' }
} finally {
  Pop-Location
}
