[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$Serial,
  [ValidateRange(5, 200)]
  [int]$Samples = 30,
  [ValidateRange(0, 50)]
  [int]$Warmup = 5,
  [string]$OutputRoot,
  [switch]$JsonCompatibility
)

$ErrorActionPreference = 'Stop'
$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$auExecutable = Join-Path $scriptRoot '..\crates\android-use\target\release\au.exe'
$pipeName = 'codex-android-use-v1'
$protocolVersion = 1
$maxProtocolFrame = 1MB
$utf8 = [System.Text.Encoding]::UTF8
if (-not (Test-Path -LiteralPath $auExecutable -PathType Leaf)) {
  throw "Release au.exe is missing: $auExecutable"
}

if ([string]::IsNullOrWhiteSpace($OutputRoot)) {
  $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
  $OutputRoot = Join-Path $env:LOCALAPPDATA "Codex\android-use\artifacts\benchmarks\$stamp"
}
New-Item -ItemType Directory -Force -Path $OutputRoot | Out-Null

function ConvertTo-WindowsArgument([string]$argument) {
  if ($argument.Length -gt 0 -and $argument -notmatch '[\s"]') {
    return $argument
  }
  $builder = [System.Text.StringBuilder]::new()
  [void]$builder.Append('"')
  $backslashes = 0
  foreach ($character in $argument.ToCharArray()) {
    if ($character -eq '\') {
      $backslashes++
      continue
    }
    if ($character -eq '"') {
      $count = (2 * $backslashes) + 1
      if ($count -gt 0) {
        [void]$builder.Append((('\' * $count) -join ''))
      }
      [void]$builder.Append('"')
      $backslashes = 0
      continue
    }
    if ($backslashes -gt 0) {
      [void]$builder.Append((('\' * $backslashes) -join ''))
      $backslashes = 0
    }
    [void]$builder.Append($character)
  }
  if ($backslashes -gt 0) {
    [void]$builder.Append((('\' * (2 * $backslashes)) -join ''))
  }
  [void]$builder.Append('"')
  return $builder.ToString()
}

function Invoke-Au([string[]]$Arguments) {
  $info = [System.Diagnostics.ProcessStartInfo]::new()
  $info.FileName = $auExecutable
  $info.UseShellExecute = $false
  $info.RedirectStandardOutput = $true
  $info.RedirectStandardError = $true
  $info.Arguments = (@($Arguments) | ForEach-Object {
      ConvertTo-WindowsArgument ([string]$_)
    }) -join ' '
  $process = [System.Diagnostics.Process]::new()
  $process.StartInfo = $info
  $timer = [System.Diagnostics.Stopwatch]::StartNew()
  if (-not $process.Start()) {
    throw 'Could not start au.exe'
  }
  if (-not $process.WaitForExit(30000)) {
    $process.Kill()
    $process.WaitForExit()
    throw 'au.exe exceeded the 30 second benchmark deadline'
  }
  $timer.Stop()
  [pscustomobject]@{
    Milliseconds = $timer.Elapsed.TotalMilliseconds
    ExitCode = $process.ExitCode
    Stdout = $process.StandardOutput.ReadToEnd()
    Stderr = $process.StandardError.ReadToEnd()
  }
}

function Read-Exact([System.IO.Stream]$Stream, [int]$Count) {
  $bytes = New-Object byte[] $Count
  $offset = 0
  while ($offset -lt $Count) {
    $read = $Stream.Read($bytes, $offset, $Count - $offset)
    if ($read -le 0) {
      throw 'daemon pipe closed before a full frame was read'
    }
    $offset += $read
  }
  return ,$bytes
}

function Add-U16([System.Collections.Generic.List[byte]]$Buffer, [uint16]$Value) {
  $Buffer.AddRange([BitConverter]::GetBytes($Value))
}

function Add-U64([System.Collections.Generic.List[byte]]$Buffer, [uint64]$Value) {
  $Buffer.AddRange([BitConverter]::GetBytes($Value))
}

function Add-String([System.Collections.Generic.List[byte]]$Buffer, [string]$Value) {
  $bytes = $utf8.GetBytes($Value)
  if ($bytes.Length -gt [uint16]::MaxValue) {
    throw 'native benchmark string exceeds 65535 bytes'
  }
  Add-U16 $Buffer ([uint16]$bytes.Length)
  $Buffer.AddRange($bytes)
}

function New-NativeRequestPayload([string]$Kind, [string[]]$Arguments, [int64]$Id) {
  $payload = [System.Collections.Generic.List[byte]]::new()
  $payload.AddRange([byte[]](0x41, 0x55, 0x32, 0x00, 0x01))
  Add-U16 $payload ([uint16]$protocolVersion)
  Add-U64 $payload ([uint64]$Id)
  switch ($Kind) {
    'hello' {
      $payload.Add(1)
      Add-String $payload 'android-use-benchmark'
    }
    'execute' {
      if ($Arguments.Count -gt [uint16]::MaxValue) { throw 'too many native benchmark arguments' }
      $payload.Add(2)
      Add-U16 $payload ([uint16]$Arguments.Count)
      foreach ($argument in $Arguments) { Add-String $payload ([string]$argument) }
    }
    'stop' { $payload.Add(3) }
    default { throw "unknown native benchmark request kind: $Kind" }
  }
  return ,$payload.ToArray()
}

function Read-U16([byte[]]$Bytes, [ref]$Offset) {
  $value = [BitConverter]::ToUInt16($Bytes, $Offset.Value)
  $Offset.Value += 2
  return $value
}

function Read-U32([byte[]]$Bytes, [ref]$Offset) {
  $value = [BitConverter]::ToUInt32($Bytes, $Offset.Value)
  $Offset.Value += 4
  return $value
}

function Read-U64([byte[]]$Bytes, [ref]$Offset) {
  $value = [BitConverter]::ToUInt64($Bytes, $Offset.Value)
  $Offset.Value += 8
  return $value
}

function Read-NativeString([byte[]]$Bytes, [ref]$Offset) {
  $length = [int](Read-U16 $Bytes $Offset)
  if ($length -lt 0 -or $Offset.Value + $length -gt $Bytes.Length) {
    throw 'truncated native benchmark string'
  }
  $value = $utf8.GetString($Bytes, $Offset.Value, $length)
  $Offset.Value += $length
  return $value
}

function Read-NativeResponse([System.IO.Stream]$Stream) {
  $header = Read-Exact $Stream 4
  $length = [BitConverter]::ToUInt32($header, 0)
  if ($length -lt 1 -or $length -gt $maxProtocolFrame) {
    throw "invalid native daemon response frame length $length"
  }
  $payload = Read-Exact $Stream ([int]$length)
  if ($payload.Length -lt 16 -or [Text.Encoding]::ASCII.GetString($payload, 0, 4) -ne "AU2`0" -or $payload[4] -ne 2) {
    throw 'invalid native daemon response header'
  }
  $offset = 5
  $version = Read-U16 $payload ([ref]$offset)
  $id = Read-U64 $payload ([ref]$offset)
  $status = $payload[$offset]
  $offset++
  if ($status -eq 1) {
    $dataLength = [int](Read-U32 $payload ([ref]$offset))
    if ($dataLength -lt 1 -or $offset + $dataLength -gt $payload.Length) {
      throw 'invalid native daemon response data length'
    }
    $data = $utf8.GetString($payload, $offset, $dataLength) | ConvertFrom-Json
    return [pscustomobject]@{ version=$version; id=$id; ok=$true; data=$data }
  }
  if ($status -eq 0) {
    return [pscustomobject]@{ version=$version; id=$id; ok=$false; error=[pscustomobject]@{code=(Read-NativeString $payload ([ref]$offset)); message=(Read-NativeString $payload ([ref]$offset))} }
  }
  throw 'invalid native daemon response status'
}

function Invoke-DaemonRequest([string]$Kind, [string[]]$Arguments, [int64]$Id) {
  if ($JsonCompatibility) {
    $body = [ordered]@{
      version = $protocolVersion
      id = $Id
      kind = $Kind
    }
    if ($Kind -eq 'hello') {
      $body.client_version = 'android-use-benchmark'
    } else {
      $body.argv = @($Arguments)
    }
    $payload = $utf8.GetBytes(($body | ConvertTo-Json -Compress -Depth 8))
  } else {
    $payload = New-NativeRequestPayload $Kind $Arguments $Id
  }
  if ($payload.Length -gt $maxProtocolFrame) {
    throw 'benchmark request exceeded the daemon frame limit'
  }
  $frame = New-Object byte[] (4 + $payload.Length)
  [Array]::Copy([BitConverter]::GetBytes([uint32]$payload.Length), 0, $frame, 0, 4)
  [Array]::Copy($payload, 0, $frame, 4, $payload.Length)

  $client = [System.IO.Pipes.NamedPipeClientStream]::new(
    '.',
    $pipeName,
    [System.IO.Pipes.PipeDirection]::InOut,
    [System.IO.Pipes.PipeOptions]::None
  )
  $timer = [System.Diagnostics.Stopwatch]::StartNew()
  try {
    $client.Connect(5000)
    $client.Write($frame, 0, $frame.Length)
    $client.Flush()
    if ($JsonCompatibility) {
      $header = Read-Exact $client 4
      $length = [BitConverter]::ToUInt32($header, 0)
      if ($length -lt 1 -or $length -gt $maxProtocolFrame) {
        throw "invalid daemon response frame length $length"
      }
      $response = $utf8.GetString((Read-Exact $client ([int]$length))) | ConvertFrom-Json
    } else {
      $response = Read-NativeResponse $client
    }
    $timer.Stop()
    if ($response.version -ne $protocolVersion) {
      throw "daemon response protocol version $($response.version) is incompatible"
    }
    if (-not $response.ok) {
      throw ("daemon request failed: " + ($response | ConvertTo-Json -Compress -Depth 8))
    }
    return [pscustomobject]@{
      Milliseconds = $timer.Elapsed.TotalMilliseconds
      Response = $response
    }
  } finally {
    $client.Dispose()
  }
}

function Get-Percentile([double[]]$values, [double]$percentile) {
  $ordered = @($values | Sort-Object)
  if ($ordered.Count -eq 0) {
    return 0.0
  }
  $rank = ($ordered.Count - 1) * $percentile
  $lower = [Math]::Floor($rank)
  $upper = [Math]::Ceiling($rank)
  if ($lower -eq $upper) {
    return [double]$ordered[$lower]
  }
  $fraction = $rank - $lower
  return ([double]$ordered[$lower]) + (([double]$ordered[$upper] - [double]$ordered[$lower]) * $fraction)
}

function Measure-Au([string]$Name, [string[]]$Arguments) {
  Write-Host ("benchmark=" + $Name + " warmup")
  for ($index = 0; $index -lt $Warmup; $index++) {
    $warm = Invoke-Au $Arguments
    if ($warm.ExitCode -ne 0) {
      throw ("Warmup failed for " + $Name + ": " + $warm.Stdout + $warm.Stderr)
    }
  }
  $sampleValues = New-Object System.Collections.Generic.List[double]
  $proof = ''
  Write-Host ("benchmark=" + $Name + " samples=" + $Samples)
  for ($index = 0; $index -lt $Samples; $index++) {
    $result = Invoke-Au $Arguments
    if ($result.ExitCode -ne 0) {
      throw ("Sample failed for " + $Name + ": " + $result.Stdout + $result.Stderr)
    }
    $sampleValues.Add([double]$result.Milliseconds)
    if ($index -eq 0) {
      $proof = $result.Stdout.Trim()
    }
  }
  [pscustomobject]@{
    name = $Name
    samples = $Samples
    warmup = $Warmup
    p50_ms = [Math]::Round((Get-Percentile $sampleValues.ToArray() 0.50), 3)
    p95_ms = [Math]::Round((Get-Percentile $sampleValues.ToArray() 0.95), 3)
    first_proof = $proof
  }
}

function Measure-DaemonRequest([string]$Name, [string]$Kind, [string[]]$Arguments) {
  Write-Host ("benchmark=" + $Name + " warmup mode=named-pipe")
  for ($index = 0; $index -lt $Warmup; $index++) {
    [void](Invoke-DaemonRequest $Kind $Arguments $index)
  }
  $sampleValues = New-Object System.Collections.Generic.List[double]
  $proof = ''
  Write-Host ("benchmark=" + $Name + " samples=" + $Samples + " mode=named-pipe")
  for ($index = 0; $index -lt $Samples; $index++) {
    $result = Invoke-DaemonRequest $Kind $Arguments ($index + 1000)
    $sampleValues.Add([double]$result.Milliseconds)
    if ($index -eq 0) {
      $proof = if ($null -eq $result.Response.data) {
        ($result.Response | ConvertTo-Json -Compress -Depth 8)
      } else {
        ($result.Response.data | ConvertTo-Json -Compress -Depth 8)
      }
    }
  }
  [pscustomobject]@{
    name = $Name
    samples = $Samples
    warmup = $Warmup
    measurement = 'direct named pipe; no au.exe startup per sample'
    p50_ms = [Math]::Round((Get-Percentile $sampleValues.ToArray() 0.50), 3)
    p95_ms = [Math]::Round((Get-Percentile $sampleValues.ToArray() 0.95), 3)
    first_proof = $proof
  }
}

$twenty = (1..20 | ForEach-Object { 'w 0' }) -join '; '
$metrics = @(
  (Measure-Au 'cold_help' @('help'))
)
Write-Host 'benchmark=daemon_start'
$daemonStart = Invoke-Au @('-s', $Serial, 'daemon', 'start')
if ($daemonStart.ExitCode -ne 0) {
  throw ("Could not start the daemon: " + $daemonStart.Stdout + $daemonStart.Stderr)
}
$metrics += @(
  (Measure-DaemonRequest 'daemon_ping' 'hello' @()),
  (Measure-DaemonRequest 'persistent_batch_noop' 'execute' @('-s', $Serial, 'b', 'w 0')),
  (Measure-DaemonRequest 'persistent_batch_20' 'execute' @('-s', $Serial, 'b', $twenty)),
  (Measure-DaemonRequest 'persistent_batch_20_zero_delay' 'execute' @('-s', $Serial, '--delay', '0', 'b', $twenty)),
  (Measure-Au 'direct_status_no_daemon' @('--no-daemon', '-s', $Serial, 'st'))
)

$daemonStatus = Invoke-Au @('-j', '-s', $Serial, 'daemon', 'status')
if ($daemonStatus.ExitCode -ne 0) {
  throw "Could not inspect daemon status: $($daemonStatus.Stdout)$($daemonStatus.Stderr)"
}
$daemonJson = $daemonStatus.Stdout | ConvertFrom-Json
$daemonData = $daemonJson.data.data
$daemonPid = [int]$daemonData.handshake.pid
if ($daemonPid -le 0) {
  throw 'Daemon status did not return a valid handshake PID'
}
$memorySamples = New-Object System.Collections.Generic.List[double]
for ($index = 0; $index -lt [Math]::Max(5, $Warmup); $index++) {
  $process = Get-Process -Id $daemonPid -ErrorAction Stop
  $memorySamples.Add([double]$process.WorkingSet64)
  Start-Sleep -Milliseconds 50
}
$metrics += [pscustomobject]@{
  name = 'daemon_idle_working_set'
  samples = $memorySamples.Count
  warmup = 0
  p50_bytes = [Math]::Round((Get-Percentile $memorySamples.ToArray() 0.50), 0)
  p95_bytes = [Math]::Round((Get-Percentile $memorySamples.ToArray() 0.95), 0)
}

$binary = Get-Item -LiteralPath $auExecutable
$transport = if ($Serial -match '^adb-.*_adb-tls-connect\._tcp$') {
  'mdns'
} elseif ($Serial -match '^\d{1,3}(?:\.\d{1,3}){3}:\d+$') {
  'wifi'
} else {
  'usb-or-local'
}
$report = [pscustomobject]@{
  generated_at = (Get-Date).ToUniversalTime().ToString('o')
  serial = $Serial
  transport = $transport
  binary = [pscustomobject]@{
    path = $binary.FullName
    bytes = $binary.Length
    sha256 = (Get-FileHash -LiteralPath $binary.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  }
  metrics = $metrics
  measurement_notes = @(
    "daemon_ping and persistent_batch metrics use direct framed named-pipe requests ($([string]::Join('', @('native AU2', $(if ($JsonCompatibility) { ' with JSON compatibility' } else { '' })) ))); au.exe startup is excluded."
    'cold_help and direct_status_no_daemon intentionally include au.exe startup.'
  )
  output_root = (Resolve-Path -LiteralPath $OutputRoot).Path
}
$reportPath = Join-Path $OutputRoot 'report.json'
$report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $reportPath -Encoding UTF8
$report | ConvertTo-Json -Depth 8
Write-Output "report=$reportPath"
