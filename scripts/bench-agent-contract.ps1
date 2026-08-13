[CmdletBinding()]
param(
  [string]$Serial,
  [ValidateRange(3, 200)]
  [int]$Samples = 20,
  [ValidateRange(0, 20)]
  [int]$Warmup = 3,
  [string]$Output = (Join-Path $PSScriptRoot '..\artifacts\agent-contract-bench.json')
)

$ErrorActionPreference = 'Stop'
$au = Join-Path $PSScriptRoot '..\target\release\au.exe'
if (-not (Test-Path -LiteralPath $au -PathType Leaf)) {
  throw "Release au.exe is missing: $au"
}

function ConvertTo-WindowsArgument([string]$Value) {
  if ($Value.Length -gt 0 -and $Value -notmatch '[\s"]') { return $Value }
  return '"' + ($Value -replace '(\\*)"', '$1$1\\"' -replace '(\\+)$', '$1$1') + '"'
}

$info = [Diagnostics.ProcessStartInfo]::new()
$info.FileName = $au
$info.Arguments = 'serve --jsonl'
$info.UseShellExecute = $false
$info.CreateNoWindow = $true
$info.RedirectStandardInput = $true
$info.RedirectStandardOutput = $true
$info.RedirectStandardError = $true
$server = [Diagnostics.Process]::Start($info)
$stderr = $server.StandardError.ReadToEndAsync()
$sequence = 0

function Invoke-Contract([string]$Method, [hashtable]$Params) {
  $script:sequence++
  if (-not [string]::IsNullOrWhiteSpace($Serial)) {
    $Params.device = @{ serial = $Serial }
  }
  $request = @{ v = 2; id = "bench-$($script:sequence)"; method = $Method; params = $Params } |
    ConvertTo-Json -Compress -Depth 16
  $timer = [Diagnostics.Stopwatch]::StartNew()
  $server.StandardInput.WriteLine($request)
  $server.StandardInput.Flush()
  $response = $server.StandardOutput.ReadLine()
  $timer.Stop()
  if ([string]::IsNullOrWhiteSpace($response)) { throw 'Contract server returned no response' }
  $parsed = $response | ConvertFrom-Json
  if (-not $parsed.ok) { throw "Contract request failed: $response" }
  [pscustomobject]@{
    milliseconds = $timer.Elapsed.TotalMilliseconds
    request_bytes = [Text.Encoding]::UTF8.GetByteCount($request)
    response_bytes = [Text.Encoding]::UTF8.GetByteCount($response)
    response = $response
  }
}

function Get-Percentile([double[]]$Values, [double]$Percentile) {
  $ordered = @($Values | Sort-Object)
  $rank = ($ordered.Count - 1) * $Percentile
  $lower = [Math]::Floor($rank)
  $upper = [Math]::Ceiling($rank)
  if ($lower -eq $upper) { return [double]$ordered[$lower] }
  [double]$ordered[$lower] + (([double]$ordered[$upper] - [double]$ordered[$lower]) * ($rank - $lower))
}

$cases = @(
  @{ name = 'status'; method = 'android.status'; params = @{} },
  @{ name = 'choices-object'; method = 'android.observe'; params = @{ mode = 'choices'; budget = @{ nodes = 64; bytes = 32768 } } },
  @{ name = 'choices-dense'; method = 'android.observe'; params = @{ mode = 'choices'; encoding = 'dense'; budget = @{ nodes = 64; bytes = 32768 } } },
  @{ name = 'query'; method = 'android.observe'; params = @{ mode = 'query'; query = 'clickable=true#0'; budget = @{ bytes = 4096 } } }
)

try {
  $results = foreach ($case in $cases) {
    for ($index = 0; $index -lt $Warmup; $index++) {
      [void](Invoke-Contract $case.method $case.params.Clone())
    }
    $measurements = for ($index = 0; $index -lt $Samples; $index++) {
      Invoke-Contract $case.method $case.params.Clone()
    }
    $times = [double[]]@($measurements.milliseconds)
    [pscustomobject]@{
      name = $case.name
      samples = $Samples
      p50_ms = [Math]::Round((Get-Percentile $times 0.50), 3)
      p95_ms = [Math]::Round((Get-Percentile $times 0.95), 3)
      mean_ms = [Math]::Round(($times | Measure-Object -Average).Average, 3)
      mean_request_bytes = [Math]::Round(($measurements.request_bytes | Measure-Object -Average).Average, 0)
      mean_response_bytes = [Math]::Round(($measurements.response_bytes | Measure-Object -Average).Average, 0)
      token_count = $null
      token_note = 'response bytes are exact; model tokenizer is intentionally not guessed'
    }
  }
  $objectBytes = ($results | Where-Object name -eq 'choices-object').mean_response_bytes
  $denseBytes = ($results | Where-Object name -eq 'choices-dense').mean_response_bytes
  $payload = [pscustomobject]@{
    schema = 2
    contract_version = 2
    transport = 'one persistent local JSONL process with warm selection/helper/CDP pools'
    serial_supplied = -not [string]::IsNullOrWhiteSpace($Serial)
    samples = $Samples
    warmup = $Warmup
    dense_response_reduction_percent = if ($objectBytes -gt 0) { [Math]::Round((1 - ($denseBytes / $objectBytes)) * 100, 2) } else { $null }
    cases = $results
  }
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Output) | Out-Null
  $payload | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $Output -Encoding UTF8
  $payload | ConvertTo-Json -Depth 8
  Write-Output "report=$Output"
} finally {
  if (-not $server.HasExited) {
    $server.StandardInput.Close()
    if (-not $server.WaitForExit(5000)) { $server.Kill() }
  }
  [void]$stderr.GetAwaiter().GetResult()
}
