[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$Serial,
  [ValidateRange(5, 120)]
  [int]$Minutes = 15,
  [Parameter(Mandatory = $true)]
  [switch]$AllowAppLifecycle,
  [Parameter(Mandatory = $true)]
  [switch]$AllowDownloadCleanup,
  [switch]$AllowSystemUi,
  [switch]$AllowRotation,
  [string]$AuPath,
  [string]$OutputRoot
)

$ErrorActionPreference = 'Stop'
if (-not $AllowAppLifecycle) { throw 'Pass -AllowAppLifecycle to authorize fixture install, stop/start, and uninstall.' }
if (-not $AllowDownloadCleanup) { throw 'Pass -AllowDownloadCleanup to authorize one deterministic download and its exact cleanup.' }

$root = Split-Path -Parent $PSScriptRoot
$au = if ([string]::IsNullOrWhiteSpace($AuPath)) {
  Join-Path $root 'target\release\au.exe'
} else {
  [string](Resolve-Path -LiteralPath $AuPath -ErrorAction Stop)
}
$adb = Join-Path $env:LOCALAPPDATA 'Android\Sdk\platform-tools\adb.exe'
$fixtureApk = Join-Path $root 'android\aubridge\fixture\build\outputs\apk\release\fixture-release.apk'
$webRoot = Join-Path $root 'crates\android-use\tests'
$fixturePackage = 'dev.codex.aubench'
$downloadName = 'au-agentic-download.txt'
$downloadPath = "/sdcard/Download/$downloadName"
$runId = 'agentic-' + [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
if ([string]::IsNullOrWhiteSpace($OutputRoot)) {
  $OutputRoot = Join-Path $env:LOCALAPPDATA "Codex\android-use\artifacts\benchmarks\$runId"
}

foreach ($required in @($au, $adb, $fixtureApk, (Join-Path $webRoot 'fixture-web.html'), (Join-Path $webRoot 'fixture-download.txt'))) {
  if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw "Required benchmark file is missing: $required" }
}

function ConvertTo-WindowsArgument([string]$Value) {
  if ($Value.Length -gt 0 -and $Value -notmatch '[\s"]') { return $Value }
  return '"' + ($Value -replace '(\\*)"', '$1$1\\"' -replace '(\\+)$', '$1$1') + '"'
}

function Start-CapturedProcess([string]$FileName, [string[]]$Arguments) {
  $info = [Diagnostics.ProcessStartInfo]::new()
  $info.FileName = $FileName
  $info.Arguments = (@($Arguments) | ForEach-Object { ConvertTo-WindowsArgument ([string]$_) }) -join ' '
  $info.UseShellExecute = $false
  $info.CreateNoWindow = $true
  $info.RedirectStandardInput = $true
  $info.RedirectStandardOutput = $true
  $info.RedirectStandardError = $true
  $process = [Diagnostics.Process]::Start($info)
  $process | Add-Member -NotePropertyName ErrorDrain -NotePropertyValue $process.StandardError.ReadToEndAsync()
  return $process
}

function Stop-CapturedProcess($Process) {
  if ($null -eq $Process) { return }
  if (-not $Process.HasExited) {
    try { $Process.StandardInput.Close() } catch { }
    if (-not $Process.WaitForExit(5000)) {
      # The child can exit between WaitForExit and Kill while its redirected
      # stderr task is draining. Treat that race as an already-clean process.
      try {
        if (-not $Process.HasExited) {
          $Process.Kill()
          $Process.WaitForExit()
        }
      } catch [InvalidOperationException] { }
    }
  }
  try { [void]$Process.ErrorDrain.GetAwaiter().GetResult() } catch { }
}

function Read-BoundedProcessLine($Process, [string]$Name, [int]$TimeoutMs = 30000) {
  $read = $Process.StandardOutput.ReadLineAsync()
  if (-not $read.Wait($TimeoutMs)) {
    throw "$Name response timed out after $TimeoutMs ms"
  }
  return $read.GetAwaiter().GetResult()
}

function Invoke-AuJson([string[]]$Arguments) {
  $info = [Diagnostics.ProcessStartInfo]::new()
  $info.FileName = $au
  $info.Arguments = (@($Arguments) | ForEach-Object { ConvertTo-WindowsArgument ([string]$_) }) -join ' '
  $info.UseShellExecute = $false
  $info.CreateNoWindow = $true
  $info.RedirectStandardOutput = $true
  $info.RedirectStandardError = $true
  $process = [Diagnostics.Process]::Start($info)
  $stdout = $process.StandardOutput.ReadToEndAsync()
  $stderr = $process.StandardError.ReadToEndAsync()
  if (-not $process.WaitForExit(60000)) { $process.Kill(); throw "au timed out: $($Arguments -join ' ')" }
  $out = $stdout.GetAwaiter().GetResult().Trim()
  $err = $stderr.GetAwaiter().GetResult().Trim()
  if ($process.ExitCode -ne 0) { throw "au failed: $out $err" }
  return $out | ConvertFrom-Json
}

function Invoke-AuJsonRetry([string[]]$Arguments, [int]$Attempts = 8) {
  $last = $null
  for ($attempt = 0; $attempt -lt $Attempts; $attempt++) {
    try { return Invoke-AuJson $Arguments } catch {
      $last = $_
      if ($_.Exception.Message -notmatch 'E_CDP|E_DEVICE|not ready') { throw }
      Start-Sleep -Milliseconds 300
    }
  }
  throw $last
}

function Get-ActionData($Envelope) {
  if ($null -ne $Envelope.data -and $null -ne $Envelope.data.data) { return $Envelope.data.data }
  if ($null -ne $Envelope.data) { return $Envelope.data }
  return $Envelope
}

function Test-ExactDeviceOnline {
  try {
    $probe = Get-ActionData (Invoke-AuJson @('--no-daemon', '-j', '-s', $Serial, 'st'))
    return $probe.hardware_serial -eq $Serial -and $probe.state -eq 'device'
  } catch {
    return $false
  }
}

function Repair-ExactDeviceTransport {
  if ($script:metrics.transport_repairs -ge 3) { return $false }
  $script:metrics.transport_repairs++
  try {
    [void](Invoke-AuJson @('--no-daemon', '-j', '-s', $Serial, 'doctor', '--repair'))
    for ($attempt = 0; $attempt -lt 20; $attempt++) {
      if (Test-ExactDeviceOnline) { return $true }
      $delay = [Math]::Min(2000, 250 * [Math]::Pow(2, [Math]::Min($attempt, 3)))
      Start-Sleep -Milliseconds ([int]$delay)
    }
  } catch {
    Add-BenchmarkEvent ([pscustomobject]@{
      cycle = [int]$script:metrics.cycles
      at = (Get-Date).ToUniversalTime().ToString('o')
      kind = 'transport_repair_failed'
      error = $_.Exception.Message
    })
  }
  return $false
}

function Test-RemoteFile([string]$Path) {
  if (-not (Test-ExactDeviceOnline)) { throw 'exact benchmark device is offline while checking a remote file' }
  & $adb -s $Serial shell test -f $Path 2>$null
  $exit = $LASTEXITCODE
  if ($exit -eq 0) { return $true }
  if ($exit -eq 1) { return $false }
  throw "remote file probe failed with adb exit $exit"
}

$status = Get-ActionData (Invoke-AuJson @('--no-daemon', '-j', '-s', $Serial, 'st'))
if ($status.hardware_serial -ne $Serial) { throw 'Serial must be the exact enrolled hardware identity for this destructive benchmark.' }
if ($status.state -ne 'device') { throw 'Exact benchmark device is not authorized and online.' }

$existingFixture = & $adb -s $Serial shell pm path $fixturePackage 2>$null
if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace(($existingFixture -join ''))) {
  throw "Fixture package already exists; refusing to overwrite or later uninstall it: $fixturePackage"
}
if (Test-RemoteFile $downloadPath) {
  throw "Benchmark download already exists; refusing to overwrite or delete it: $downloadPath"
}

New-Item -ItemType Directory -Force -Path $OutputRoot | Out-Null
$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$listener.Start()
$port = ([Net.IPEndPoint]$listener.LocalEndpoint).Port
$listener.Stop()
$python = (Get-Command python -ErrorAction Stop).Source
$http = $null
$contract = $null
$pipe = $null
$fixtureInstalled = $false
$reverseCreated = $false
$downloadCreated = $false
$benchmarkTab = $null
$cleanup = [ordered]@{
  fixture_uninstalled = $false
  download_removed = $false
  tab_closed = $false
  reverse_removed = $false
  rotation_restored = -not $AllowRotation
  foreground_restored = $false
}
$metrics = [ordered]@{
  cycles = 0; app_episodes = 0; browser_episodes = 0; contract_calls = 0; pipe_calls = 0
  committed_operations = 0; device_plan_runs = 0; proof_runs = 0; host_step_plans = 0
  verified = 0; stale_recoveries = 0; errors = 0
  observations = 0; dense_response_bytes = 0; downloads = 0; app_restarts = 0
  transport_disconnects = 0; transport_repairs = 0; max_consecutive_errors = 0; dropped_events = 0
  system_ui_episodes = 0; rotation_injections = 0
}
$events = [System.Collections.Generic.List[object]]::new()
$contractSequence = 0
$consecutiveErrors = 0
$abortedReason = $null
$fatalError = $null
$summary = $null
$started = $null
$preForeground = $null
$initialAccelerometerRotation = $null
$initialUserRotation = $null

function Add-BenchmarkEvent([object]$Event) {
  if ($script:events.Count -lt 128) {
    $script:events.Add($Event)
  } else {
    $script:metrics.dropped_events++
  }
}

function Send-Contract([string]$Method, [hashtable]$Params) {
  $script:contractSequence++
  $Params.device = @{ serial = $Serial; endpoint = $Serial }
  $request = @{ v = 2; id = "$runId-c$($script:contractSequence)"; method = $Method; params = $Params } |
    ConvertTo-Json -Compress -Depth 20
  $timer = [Diagnostics.Stopwatch]::StartNew()
  $contract.StandardInput.WriteLine($request)
  $contract.StandardInput.Flush()
  $line = Read-BoundedProcessLine $contract 'contract' 30000
  $timer.Stop()
  if ([string]::IsNullOrWhiteSpace($line)) { throw 'contract server returned no response' }
  $response = $line | ConvertFrom-Json
  $script:metrics.contract_calls++
  if (-not $response.ok) {
    $code = [string]$response.error.code
    if ($code -eq 'E_STALE') { $script:metrics.stale_recoveries++ }
    throw "$code $([string]$response.error.message)"
  }
  [pscustomobject]@{ result = $response.result; bytes = [Text.Encoding]::UTF8.GetByteCount($line); milliseconds = $timer.Elapsed.TotalMilliseconds }
}

function Send-Pipe([string]$Command, [string[]]$Arguments) {
  $request = @{ c = $Command; a = @($Arguments) } | ConvertTo-Json -Compress -Depth 8
  $pipe.StandardInput.WriteLine($request)
  $pipe.StandardInput.Flush()
  $line = Read-BoundedProcessLine $pipe 'typed pipe' 30000
  if ([string]::IsNullOrWhiteSpace($line)) { throw 'typed pipe returned no response' }
  $response = $line | ConvertFrom-Json
  $script:metrics.pipe_calls++
  if ($response.ok -eq $false) { throw "typed pipe failed: $line" }
  return $response
}

function Observe-Dense {
  $observation = Send-Contract 'android.observe' @{ mode = 'choices'; encoding = 'dense'; budget = @{ nodes = 64; bytes = 16384 } }
  $script:metrics.observations++
  $script:metrics.dense_response_bytes += $observation.bytes
  return $observation.result
}

function Observe-Object {
  $observation = Send-Contract 'android.observe' @{ mode = 'choices'; encoding = 'object'; budget = @{ nodes = 64; bytes = 32768 } }
  $script:metrics.observations++
  return $observation.result
}

function Observe-StableFixture {
  $previous = $null
  for ($attempt = 0; $attempt -lt 12; $attempt++) {
    $candidate = Observe-Dense
    $labels = @($candidate.d.n | ForEach-Object { [string]$_[1] })
    if ($labels -contains 'AU agentic benchmark fixture') {
      if ($null -ne $previous -and [uint64]$previous.g -eq [uint64]$candidate.g) {
        return $candidate
      }
      $previous = $candidate
    } else {
      $previous = $null
    }
    Start-Sleep -Milliseconds 75
  }
  throw 'fixture accessibility generation did not settle'
}

function Run-FixtureEpisode([int]$Cycle) {
  [void](Send-Pipe 'app' @('stop', $fixturePackage))
  [void](Send-Pipe 'app' @('start', $fixturePackage, '.MainActivity'))
  $script:metrics.app_restarts++
  $observation = Observe-StableFixture
  $value = "cycle-$Cycle-$([DateTimeOffset]::UtcNow.ToUnixTimeSeconds())"
  $steps = @(
    @{ op = 'wait'; target = @{ selector = 'desc=Benchmark text input#0' }; timeout_ms = 3000 },
    @{ op = 'set'; target = @{ selector = 'desc=Benchmark text input#0' }; text = $value },
    @{ op = 'tap'; target = @{ selector = 'desc=Benchmark submit#0' } },
    @{ op = 'assert'; target = @{ selector = "text=Submitted: $value#0" }; timeout_ms = 3000 },
    @{ op = 'tap'; target = @{ selector = 'desc=Benchmark dialog#0' } },
    @{ op = 'wait'; target = @{ selector = 'text=Benchmark confirmation#0' }; timeout_ms = 3000 },
    @{ op = 'tap'; target = @{ selector = 'text=CONFIRM,clickable=true#0' } },
    @{ op = 'assert'; target = @{ selector = 'text=Dialog confirmed#0' }; timeout_ms = 3000 },
    @{ op = 'scroll'; target = @{ selector = 'scrollable=true#0' }; direction = 'forward' }
  )
  $plan = @{
    operation_id = "$runId-app-$Cycle"
    expected_identity = $Serial
    expected_generation = [uint64]$observation.g
    deadline_ms = 12000
    max_mutations = 5
    sensitive = 'deny'
    steps = $steps
  }
  $result = $null
  for ($attempt = 0; $attempt -lt 4; $attempt++) {
    $plan.expected_generation = [uint64]$observation.g
    $plan.operation_id = if ($attempt -eq 0) { "$runId-app-$Cycle" } else { "$runId-app-$Cycle-retry-$attempt" }
    try {
      $result = Send-Contract 'android.execute' $plan
      break
    } catch {
      if ($_.Exception.Message -notmatch '^E_STALE ' -or $attempt -eq 3) { throw }
      $observation = Observe-StableFixture
    }
  }
  if ($null -eq $result) { throw 'fixture plan produced no result' }
  if (-not $result.result.committed) { throw 'fixture plan did not commit' }
  if ($result.result.compiled -ne 'plan-run') {
    throw "fixture plan unexpectedly used host-step fallback: $($result.result.compiled)"
  }
  if ($result.result.result.c -ne $true) {
    throw "fixture device plan stopped at step $($result.result.result.failed_index) after $($result.result.result.m) mutations ($($result.result.result.e))"
  }
  $script:metrics.committed_operations += [int]$result.result.mutations
  switch ([string]$result.result.compiled) {
    'plan-run' { $script:metrics.device_plan_runs++ }
    'helper-proof' { $script:metrics.proof_runs++ }
    default { $script:metrics.host_step_plans++ }
  }
  [void](Observe-Dense)
  $script:metrics.verified += 3
  $script:metrics.app_episodes++
}

function Run-BrowserEpisode([int]$Cycle, [string]$Url) {
  [void](Send-Pipe 'app' @('start', 'com.android.chrome'))
  [void](Send-Pipe 'web' @('go', $Url))
  [void](Send-Pipe 'web' @('wait', '#name', '5000'))
  [void](Send-Pipe 'web' @('click', '#name'))
  [void](Send-Pipe 'web' @('type', "browser-$Cycle"))
  [void](Send-Pipe 'web' @('click', '#go'))
  [void](Send-Pipe 'web' @('wait', "text~Hello browser-$Cycle", '5000'))
  $page = Get-ActionData (Send-Pipe 'web' @('text'))
  $pageText = [string]$page.text
  if (-not $pageText.Contains("Hello browser-$Cycle")) { throw 'browser result text was not observed' }
  $semantic = Observe-Object
  $chromeVisible = @($semantic.data.choices | Where-Object { $_.package_name -eq 'com.android.chrome' }).Count -gt 0
  if (-not $chromeVisible) { throw 'Chrome was not the foreground accessibility package' }
  $script:metrics.verified++
  $script:metrics.browser_episodes++
}

function Run-SystemUiEpisode {
  [void](Send-Pipe 'app' @('start', 'com.android.settings'))
  for ($attempt = 0; $attempt -lt 12; $attempt++) {
    try {
      $semantic = Observe-Object
      $settingsVisible = @($semantic.data.choices | Where-Object { $_.package_name -eq 'com.android.settings' }).Count -gt 0
      if ($settingsVisible) {
        $script:metrics.system_ui_episodes++
        $script:metrics.verified++
        return
      }
    } catch {
      if ($_.Exception.Message -notmatch 'E_UI no active accessibility window|E_STALE') { throw }
    }
    Start-Sleep -Milliseconds ([Math]::Min(500, 100 + ($attempt * 50)))
  }
  throw 'Settings was not the foreground accessibility package'
}

function Get-SystemSetting([string]$Name) {
  $data = Get-ActionData (Invoke-AuJson @('--no-daemon', '-j', '-s', $Serial, 'settings', 'get', 'system', $Name))
  return ([string]$data.text).Trim()
}

function Set-SystemSetting([string]$Name, [string]$Value) {
  [void](Invoke-AuJson @('--no-daemon', '-j', '-s', $Serial, 'settings', 'put', 'system', $Name, $Value))
}

function Get-ForegroundComponent {
  $output = (& $adb -s $Serial shell dumpsys activity activities 2>$null) -join "`n"
  $match = [regex]::Match(
    $output,
    '(?m)(?:topResumedActivity|ResumedActivity|mResumedActivity|mFocusedApp)[:=].*?\su\d+\s+([A-Za-z0-9._]+/[A-Za-z0-9.$_]+)[\s}]'
  )
  if (-not $match.Success) {
    $output = (& $adb -s $Serial shell dumpsys window windows 2>$null) -join "`n"
    $match = [regex]::Match($output, '(?m)mCurrentFocus=.*?\s([A-Za-z0-9._]+/[A-Za-z0-9.$_]+)\}')
  }
  if ($match.Success) { return $match.Groups[1].Value }
  return $null
}

function Restart-BenchmarkSessions {
  Stop-CapturedProcess $script:pipe
  Stop-CapturedProcess $script:contract
  $script:pipe = $null
  $script:contract = $null
  & $adb -s $Serial reverse "tcp:$port" "tcp:$port" | Out-Null
  if ($LASTEXITCODE -ne 0) { throw 'could not restore exact benchmark ADB reverse' }
  $script:reverseCreated = $true
  $script:contract = Start-CapturedProcess $au @('serve', '--jsonl')
  $script:pipe = Start-CapturedProcess $au @('-j', '-s', $Serial, 'pipe', '--jsonl')
  if ($null -ne $script:benchmarkTab) {
    [void](Send-Pipe 'app' @('start', 'com.android.chrome'))
    [void](Send-Pipe 'web' @('use', $script:benchmarkTab))
  }
}

function Repair-BenchmarkTransport {
  if (-not (Repair-ExactDeviceTransport)) { return $false }
  try {
    Restart-BenchmarkSessions
    Add-BenchmarkEvent ([pscustomobject]@{
      cycle = [int]$script:metrics.cycles
      at = (Get-Date).ToUniversalTime().ToString('o')
      kind = 'transport_recovered'
    })
    return $true
  } catch {
    Add-BenchmarkEvent ([pscustomobject]@{
      cycle = [int]$script:metrics.cycles
      at = (Get-Date).ToUniversalTime().ToString('o')
      kind = 'session_restore_failed'
      error = $_.Exception.Message
    })
    return $false
  }
}

$preForeground = Get-ForegroundComponent
if ([string]::IsNullOrWhiteSpace($preForeground)) {
  throw 'Could not snapshot the exact foreground component before the benchmark.'
}
if ($AllowRotation) {
  $initialAccelerometerRotation = Get-SystemSetting 'accelerometer_rotation'
  $initialUserRotation = Get-SystemSetting 'user_rotation'
  if ($initialAccelerometerRotation -notmatch '^[01]$' -or $initialUserRotation -notmatch '^[0-3]$') {
    throw 'Could not snapshot valid rotation settings before the benchmark.'
  }
}
$runStarted = [DateTimeOffset]::UtcNow
$workloadCompleted = $false
$finalReady = $false
$finalTransport = $null

try {
  $http = Start-CapturedProcess $python @('-m', 'http.server', $port.ToString(), '--bind', '127.0.0.1', '--directory', $webRoot)
  $serverReady = $false
  for ($attempt = 0; $attempt -lt 30; $attempt++) {
    try {
      $probe = Invoke-WebRequest -UseBasicParsing -TimeoutSec 1 "http://127.0.0.1:$port/fixture-web.html"
      if ($probe.StatusCode -eq 200) { $serverReady = $true; break }
    } catch { Start-Sleep -Milliseconds 100 }
  }
  if (-not $serverReady) { throw 'local benchmark web server did not become ready' }

  & $adb -s $Serial reverse "tcp:$port" "tcp:$port" | Out-Null
  if ($LASTEXITCODE -ne 0) { throw 'could not create exact benchmark ADB reverse' }
  $reverseCreated = $true

  [void](Invoke-AuJson @('--no-daemon', '-j', '-s', $Serial, 'app', 'install', $fixtureApk))
  $fixtureInstalled = $true
  $contract = Start-CapturedProcess $au @('serve', '--jsonl')
  $pipe = Start-CapturedProcess $au @('-j', '-s', $Serial, 'pipe', '--jsonl')

  [void](Send-Pipe 'app' @('start', 'com.android.chrome'))
  Start-Sleep -Seconds 1
  $beforeTabs = @(Get-ActionData (Invoke-AuJsonRetry @('-j', '-s', $Serial, 'web', 'tabs'))).tabs
  $beforeIds = @($beforeTabs | ForEach-Object { [string]$_.id })
  $url = "http://127.0.0.1:$port/fixture-web.html"
  [void](Send-Pipe 'web' @('open', $url))
  Start-Sleep -Seconds 1
  $afterTabs = @(Get-ActionData (Invoke-AuJsonRetry @('-j', '-s', $Serial, 'web', 'tabs'))).tabs
  $newTabs = @($afterTabs | Where-Object { $beforeIds -notcontains [string]$_.id })
  if ($newTabs.Count -ne 1) { throw 'browser fixture did not create exactly one disposable tab; refusing to risk an existing tab' }
  $benchmarkTab = [string]$newTabs[0].id
  [void](Send-Pipe 'web' @('use', $benchmarkTab))

  Run-BrowserEpisode 0 $url
  [void](Send-Pipe 'web' @('click', '#download'))
  for ($attempt = 0; $attempt -lt 40; $attempt++) {
    if (Test-RemoteFile $downloadPath) { $downloadCreated = $true; break }
    Start-Sleep -Milliseconds 250
  }
  if (-not $downloadCreated) { throw 'deterministic browser download was not created' }
  $metrics.downloads++

  $started = [DateTimeOffset]::UtcNow
  $deadline = $started.AddMinutes($Minutes)
  while ([DateTimeOffset]::UtcNow -lt $deadline) {
    $metrics.cycles++
    $cycle = [int]$metrics.cycles
    try {
      Run-FixtureEpisode $cycle
      if (($cycle % 3) -eq 0) { Run-BrowserEpisode $cycle $url }
      if (($cycle % 5) -eq 0) {
        [void](Send-Pipe 'app' @('stop', $fixturePackage))
        [void](Send-Pipe 'app' @('start', $fixturePackage, '.MainActivity'))
        $metrics.app_restarts++
      }
      if ($AllowSystemUi -and ($cycle % 10) -eq 0) {
        Run-SystemUiEpisode
      }
      if ($AllowRotation -and ($cycle % 25) -eq 0) {
        # Settings is intentionally a separate UI class. Re-enter the
        # disposable fixture before rotating so the semantic assertion is
        # made against a stable, owned window rather than a system transition.
        [void](Send-Pipe 'app' @('start', $fixturePackage, '.MainActivity'))
        Start-Sleep -Milliseconds 400
        $alternate = ([int]$initialUserRotation + 1) % 4
        Set-SystemSetting 'accelerometer_rotation' '0'
        Set-SystemSetting 'user_rotation' ([string]$alternate)
        Start-Sleep -Milliseconds 700
        [void](Observe-Dense)
        Set-SystemSetting 'user_rotation' $initialUserRotation
        Set-SystemSetting 'accelerometer_rotation' $initialAccelerometerRotation
        Start-Sleep -Milliseconds 700
        if ((Get-SystemSetting 'accelerometer_rotation') -ne $initialAccelerometerRotation -or
            (Get-SystemSetting 'user_rotation') -ne $initialUserRotation) {
          throw 'rotation injection did not restore the exact pre-run settings'
        }
        $metrics.rotation_injections++
        $metrics.verified++
      }
      $consecutiveErrors = 0
    } catch {
      $metrics.errors++
      $consecutiveErrors++
      $metrics.max_consecutive_errors = [Math]::Max($metrics.max_consecutive_errors, $consecutiveErrors)
      $message = $_.Exception.Message
      Add-BenchmarkEvent ([pscustomobject]@{ cycle = $cycle; at = (Get-Date).ToUniversalTime().ToString('o'); kind = 'episode_error'; error = $message })
      $transportFailure = $message -match 'E_DEVICE|not connected|device .* not found|device offline|transport'
      if ($transportFailure) {
        if ($consecutiveErrors -eq 1) { $metrics.transport_disconnects++ }
        if (Repair-BenchmarkTransport) {
          $consecutiveErrors = 0
          continue
        }
      }
      if ($consecutiveErrors -ge 5) {
        $abortedReason = "five consecutive episode failures; last error: $message"
        break
      }
      $delay = [Math]::Min(5000, 250 * [Math]::Pow(2, [Math]::Min($consecutiveErrors, 4)))
      Start-Sleep -Milliseconds ([int]$delay)
      try { [void](Send-Pipe 'app' @('start', $fixturePackage, '.MainActivity')) } catch { }
    }
    if (($cycle % 5) -eq 0) {
      $remaining = [Math]::Max(0, [int]($deadline - [DateTimeOffset]::UtcNow).TotalSeconds)
      Write-Output "PROGRESS cycle=$cycle remaining_s=$remaining app=$($metrics.app_episodes) browser=$($metrics.browser_episodes) errors=$($metrics.errors)"
    }
  }
  $workloadCompleted = [string]::IsNullOrWhiteSpace($abortedReason)
} catch {
  $fatalError = $_.Exception.Message
  if ([string]::IsNullOrWhiteSpace($abortedReason)) { $abortedReason = $fatalError }
  Add-BenchmarkEvent ([pscustomobject]@{
    cycle = [int]$metrics.cycles
    at = (Get-Date).ToUniversalTime().ToString('o')
    kind = 'fatal_error'
    error = $fatalError
  })
} finally {
  Stop-CapturedProcess $pipe
  Stop-CapturedProcess $contract
  $pipe = $null
  $contract = $null
  if (-not (Test-ExactDeviceOnline)) {
    [void](Repair-ExactDeviceTransport)
  }
  if ($null -ne $benchmarkTab) {
    try {
      [void](Invoke-AuJson @('-j', '-s', $Serial, 'web', 'close', $benchmarkTab))
      $tabs = @(Get-ActionData (Invoke-AuJson @('-j', '-s', $Serial, 'web', 'tabs'))).tabs
      $cleanup.tab_closed = @($tabs | Where-Object { [string]$_.id -eq $benchmarkTab }).Count -eq 0
    } catch { }
  } else {
    $cleanup.tab_closed = $true
  }
  try {
    if (Test-RemoteFile $downloadPath) {
      [void](Invoke-AuJson @('--no-daemon', '-j', '-s', $Serial, 'file', 'rm', $downloadPath))
    }
    $cleanup.download_removed = -not (Test-RemoteFile $downloadPath)
  } catch { }
  try {
    $left = (& $adb -s $Serial shell pm path $fixturePackage 2>$null) -join ''
    if (-not [string]::IsNullOrWhiteSpace($left)) {
      [void](Invoke-AuJson @('--no-daemon', '-j', '-s', $Serial, 'app', 'uninstall', $fixturePackage))
    }
    $left = (& $adb -s $Serial shell pm path $fixturePackage 2>$null) -join ''
    $cleanup.fixture_uninstalled = [string]::IsNullOrWhiteSpace($left)
  } catch { }
  try {
    & $adb -s $Serial reverse --remove "tcp:$port" 2>$null | Out-Null
    $reverseList = (& $adb -s $Serial reverse --list 2>$null) -join "`n"
    $cleanup.reverse_removed = $reverseList -notmatch [regex]::Escape("tcp:$port")
  } catch { }
  if ($AllowRotation) {
    try {
      Set-SystemSetting 'user_rotation' $initialUserRotation
      Set-SystemSetting 'accelerometer_rotation' $initialAccelerometerRotation
      $cleanup.rotation_restored = (Get-SystemSetting 'user_rotation') -eq $initialUserRotation -and
        (Get-SystemSetting 'accelerometer_rotation') -eq $initialAccelerometerRotation
    } catch { }
  }
  try {
    $parts = $preForeground -split '/', 2
    [void](Invoke-AuJson @('--no-daemon', '-j', '-s', $Serial, 'app', 'start', $parts[0], $parts[1]))
    Start-Sleep -Milliseconds 500
    $cleanup.foreground_restored = (Get-ForegroundComponent) -eq $preForeground
  } catch { }
  try {
    $readyData = Get-ActionData (Invoke-AuJson @('--no-daemon', '-j', '-s', $Serial, 'ready'))
    $finalReady = [bool]$readyData.ready
    $statusData = Get-ActionData (Invoke-AuJson @('--no-daemon', '-j', '-s', $Serial, 'st'))
    $finalTransport = [string]$statusData.transport.kind
  } catch { }
  Stop-CapturedProcess $http
}

$durationOrigin = if ($null -ne $started) { $started } else { $runStarted }
$summary = [ordered]@{
  schema = 2
  run_id = $runId
  duration_seconds = [int]([DateTimeOffset]::UtcNow - $durationOrigin).TotalSeconds
  requested_minutes = $Minutes
  exact_identity_verified = $true
  transport = $finalTransport
  final_ready = $finalReady
  workload_completed = $workloadCompleted
  aborted_reason = $abortedReason
  fatal_error = $fatalError
  metrics = $metrics
  mean_dense_response_bytes = if ($metrics.observations -gt 0) { [Math]::Round($metrics.dense_response_bytes / $metrics.observations, 2) } else { $null }
  events = $events
  cleanup = $cleanup
}
$summary.acceptance = [ordered]@{
  no_errors = $metrics.errors -eq 0
  workload_completed = $workloadCompleted
  native_ui_exercised = $metrics.app_episodes -gt 0
  browser_ui_exercised = $metrics.browser_episodes -gt 1
  system_ui_exercised = (-not $AllowSystemUi) -or $metrics.system_ui_episodes -gt 0
  rotation_exercised = (-not $AllowRotation) -or $metrics.rotation_injections -gt 0
  all_native_plans_device_resident = $metrics.device_plan_runs -eq $metrics.app_episodes
  no_host_step_fallback = $metrics.host_step_plans -eq 0
  final_ready = [bool]$summary.final_ready
  cleanup_complete = $cleanup.fixture_uninstalled -and $cleanup.download_removed -and $cleanup.tab_closed -and
    $cleanup.reverse_removed -and $cleanup.rotation_restored -and $cleanup.foreground_restored
}
$summary.acceptance.passed = @($summary.acceptance.Values | Where-Object { $_ -ne $true }).Count -eq 0
$report = Join-Path $OutputRoot 'report.json'
$summary | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $report -Encoding UTF8
$summary | ConvertTo-Json -Depth 12
Write-Output "report=$report"
if (-not $summary.acceptance.cleanup_complete) {
  throw 'Benchmark completed but exact cleanup proof is incomplete; inspect the report.'
}
if (-not $summary.acceptance.passed) {
  throw 'Benchmark completed but one or more workload acceptance checks failed; inspect the report.'
}
