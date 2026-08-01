[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$Serial,
  [ValidateRange(5, 100)]
  [int]$Samples = 30,
  [Parameter(Mandatory = $true)]
  [string]$BaselineExe,
  [Parameter(Mandatory = $true)]
  [string]$CandidateExe,
  [Parameter(Mandatory = $true)]
  [string]$OutputRoot,
  [ValidateSet('both', 'baseline', 'candidate', 'candidate-daemon')]
  [string]$OnlyLane = 'both'
)

$ErrorActionPreference = 'Stop'
$adb = (Get-Command adb -ErrorAction Stop).Source
$selector = 'desc=AU tap target,clickable=true'
$postselector = 'text=Tapped'

function ConvertTo-WindowsArgument([string]$Argument) {
  if ($Argument.Length -gt 0 -and $Argument -notmatch '[\s"]') {
    return $Argument
  }
  $builder = [System.Text.StringBuilder]::new()
  [void]$builder.Append('"')
  $backslashes = 0
  foreach ($character in $Argument.ToCharArray()) {
    if ($character -eq '\') {
      $backslashes++
      continue
    }
    if ($character -eq '"') {
      [void]$builder.Append(('\' * ((2 * $backslashes) + 1)))
      [void]$builder.Append('"')
      $backslashes = 0
      continue
    }
    if ($backslashes -gt 0) {
      [void]$builder.Append(('\' * $backslashes))
      $backslashes = 0
    }
    [void]$builder.Append($character)
  }
  if ($backslashes -gt 0) {
    [void]$builder.Append(('\' * (2 * $backslashes)))
  }
  [void]$builder.Append('"')
  return $builder.ToString()
}

function Invoke-Process([string]$FileName, [string[]]$Arguments) {
  $info = [System.Diagnostics.ProcessStartInfo]::new()
  $info.FileName = $FileName
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
    throw "could not start $FileName"
  }
  if (-not $process.WaitForExit(30000)) {
    $process.Kill()
    $process.WaitForExit()
    throw "$FileName exceeded the 30 second action deadline"
  }
  $timer.Stop()
  [pscustomobject]@{
    ExitCode = $process.ExitCode
    Milliseconds = $timer.Elapsed.TotalMilliseconds
    Stdout = $process.StandardOutput.ReadToEnd()
    Stderr = $process.StandardError.ReadToEnd()
  }
}

function Reset-TestActivity {
  & $adb -s $Serial shell am start -n 'dev.codex.aubridge/.MainActivity' -f 0x04000000 | Out-Null
  if ($LASTEXITCODE -ne 0) { throw 'could not launch AU Bridge main activity' }
  Start-Sleep -Milliseconds 80
  & $adb -s $Serial shell input tap 640 299 | Out-Null
  if ($LASTEXITCODE -ne 0) { throw 'could not open AU deterministic test activity' }
  # Establish a stable fixture before timing the protocol. Transition waits
  # belong inside the measured operation; fixture boot is not part of F1.
  Start-Sleep -Milliseconds 500
}

function Invoke-Au([string]$Executable, [string[]]$Arguments) {
  $result = Invoke-Process $Executable $Arguments
  if ($result.ExitCode -ne 0) {
    throw "AU failed ($($result.ExitCode)): $($result.Stdout)$($result.Stderr)"
  }
  [pscustomobject]@{
    milliseconds = $result.Milliseconds
    bytes = [Text.Encoding]::UTF8.GetByteCount($result.Stdout)
    stdout = $result.Stdout.Trim()
  }
}

function Run-Baseline([string]$Executable) {
  $calls = @(
    @('--no-daemon', '-j', '-s', $Serial, 'ui', 'find', $selector),
    @('--no-daemon', '-j', '-s', $Serial, 'ui', 'tap', $selector),
    @('--no-daemon', '-j', '-s', $Serial, 'ui', 'wait', $postselector, '5000'),
    @('--no-daemon', '-j', '-s', $Serial, 'ui', 'assert', $postselector, '5000')
  )
  $timer = [System.Diagnostics.Stopwatch]::StartNew()
  $results = foreach ($call in $calls) { Invoke-Au $Executable $call }
  $timer.Stop()
  [pscustomobject]@{
    milliseconds = $timer.Elapsed.TotalMilliseconds
    cli_processes = 4
    helper_sessions = 4
    tool_calls = 4
    output_bytes = ($results | Measure-Object bytes -Sum).Sum
  }
}

function Run-Candidate([string]$Executable) {
  $timer = [System.Diagnostics.Stopwatch]::StartNew()
  $result = Invoke-Au $Executable @('--no-daemon', '-j', '-s', $Serial, 'exp', 'f1', $selector, $postselector, '5000')
  $timer.Stop()
  [pscustomobject]@{
    milliseconds = $timer.Elapsed.TotalMilliseconds
    cli_processes = 1
    helper_sessions = 1
    tool_calls = 1
    output_bytes = $result.bytes
  }
}

function Run-CandidateDaemon([string]$Executable) {
  $timer = [System.Diagnostics.Stopwatch]::StartNew()
  $result = Invoke-Au $Executable @('-j', '-s', $Serial, 'exp', 'f1', $selector, $postselector, '5000')
  $timer.Stop()
  [pscustomobject]@{
    milliseconds = $timer.Elapsed.TotalMilliseconds
    cli_processes = 1
    helper_sessions = 1
    tool_calls = 1
    output_bytes = $result.bytes
  }
}

function Percentile([double[]]$Values, [double]$Fraction) {
  $ordered = @($Values | Sort-Object)
  $rank = ($ordered.Count - 1) * $Fraction
  $lower = [Math]::Floor($rank)
  $upper = [Math]::Ceiling($rank)
  if ($lower -eq $upper) { return [double]$ordered[$lower] }
  $part = $rank - $lower
  return [double]$ordered[$lower] + (([double]$ordered[$upper] - [double]$ordered[$lower]) * $part)
}

New-Item -ItemType Directory -Force -Path $OutputRoot | Out-Null
$rows = [System.Collections.Generic.List[object]]::new()
$lanes = if ($OnlyLane -eq 'both') { @('baseline', 'candidate') } else { @($OnlyLane) }
foreach ($lane in $lanes) {
    $executable = if ($lane -eq 'baseline') { $BaselineExe } else { $CandidateExe }
  $values = [System.Collections.Generic.List[double]]::new()
  for ($index = 0; $index -lt $Samples; $index++) {
    Reset-TestActivity
    $sample = if ($lane -eq 'baseline') {
      Run-Baseline $executable
    } elseif ($lane -eq 'candidate-daemon') {
      Run-CandidateDaemon $executable
    } else {
      Run-Candidate $executable
    }
    $values.Add([double]$sample.milliseconds)
    $rows.Add([pscustomobject]@{
      lane = $lane
      sample = $index + 1
      transport = if ($Serial -match ':') { 'wifi' } else { 'usb' }
      milliseconds = [Math]::Round($sample.milliseconds, 3)
      cli_processes = $sample.cli_processes
      helper_sessions = $sample.helper_sessions
      tool_calls = $sample.tool_calls
      output_bytes = $sample.output_bytes
    })
  }
  Write-Output ("lane=$lane transport=$Serial p50_ms={0:N3} p95_ms={1:N3}" -f (Percentile $values.ToArray() .5), (Percentile $values.ToArray() .95))
}

$report = [pscustomobject]@{
  generated_at = (Get-Date).ToUniversalTime().ToString('o')
  serial = $Serial
  transport = if ($Serial -match ':') { 'wifi' } else { 'usb' }
  samples = $Samples
  fixture = [pscustomobject]@{ activity = 'dev.codex.aubridge/.TestActivity'; selector = $selector; postcondition = $postselector }
  baseline_binary = [pscustomobject]@{ path = $BaselineExe; sha256 = (Get-FileHash -LiteralPath $BaselineExe -Algorithm SHA256).Hash.ToLowerInvariant() }
  candidate_binary = [pscustomobject]@{ path = $CandidateExe; sha256 = (Get-FileHash -LiteralPath $CandidateExe -Algorithm SHA256).Hash.ToLowerInvariant() }
  samples_detail = $rows
  summary = @(
    foreach ($lane in $lanes) {
      $values = @($rows | Where-Object lane -eq $lane | Select-Object -ExpandProperty milliseconds)
      $sample = $rows | Where-Object lane -eq $lane | Select-Object -First 1
      [pscustomobject]@{ lane = $lane; p50_ms = [Math]::Round((Percentile $values .5), 3); p95_ms = [Math]::Round((Percentile $values .95), 3); tool_calls = $sample.tool_calls; helper_sessions = $sample.helper_sessions; output_bytes = $sample.output_bytes }
    }
  )
}
$reportPath = Join-Path $OutputRoot 'report.json'
$report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $reportPath -Encoding UTF8
$report | ConvertTo-Json -Depth 8
Write-Output "report=$reportPath"
