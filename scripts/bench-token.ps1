[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)]
  [string]$Serial,
  [string]$OutputRoot,
  [string]$Selector = 'desc=AU tap target,clickable=true',
  [string]$Postcondition = 'text~Tapped'
)

$ErrorActionPreference = 'Stop'
$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$au = Join-Path $scriptRoot '..\crates\android-use\target\release\au.exe'
$tokenizerRoot = Join-Path (Split-Path -Parent $scriptRoot) 'artifacts\tools\tiktoken'
if (-not (Test-Path -LiteralPath $au -PathType Leaf)) { throw "au.exe is missing: $au" }
if (-not (Test-Path -LiteralPath $tokenizerRoot -PathType Container)) { throw "tiktoken measurement environment is missing: $tokenizerRoot" }
if ([string]::IsNullOrWhiteSpace($OutputRoot)) {
  $stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
  $OutputRoot = Join-Path (Split-Path -Parent $scriptRoot) ("artifacts\token-bench\$stamp")
}
New-Item -ItemType Directory -Force -Path $OutputRoot | Out-Null

$oldPythonPath = $env:PYTHONPATH
$env:PYTHONPATH = $tokenizerRoot

function Invoke-Text([string[]]$Arguments) {
  $info = [System.Diagnostics.ProcessStartInfo]::new()
  $info.FileName = $au
  $info.UseShellExecute = $false
  $info.RedirectStandardOutput = $true
  $info.RedirectStandardError = $true
  $info.Arguments = (@($Arguments) | ForEach-Object {
      $value = [string]$_
      if ($value -notmatch '[\s"]') { return $value }
      '"' + ($value -replace '(\\*)"', '$1$1\\"' -replace '(\\+)$', '$1$1') + '"'
    }) -join ' '
  $process = [System.Diagnostics.Process]::new()
  $process.StartInfo = $info
  if (-not $process.Start()) { throw 'Could not start au.exe' }
  $stdoutTask = $process.StandardOutput.ReadToEndAsync()
  $stderrTask = $process.StandardError.ReadToEndAsync()
  if (-not $process.WaitForExit(30000)) {
    $process.Kill()
    $process.WaitForExit()
    throw 'au.exe exceeded the 30 second token measurement deadline'
  }
  $stdout = $stdoutTask.GetAwaiter().GetResult()
  $stderr = $stderrTask.GetAwaiter().GetResult()
  if ($process.ExitCode -ne 0) { throw "au failed ($($process.ExitCode)): $stdout$stderr" }
  return $stdout.TrimEnd("`r", "`n")
}

function Get-TokenCount([string]$Text) {
  $info = [System.Diagnostics.ProcessStartInfo]::new()
  $info.FileName = 'python'
  $info.UseShellExecute = $false
  $info.RedirectStandardInput = $true
  $info.RedirectStandardOutput = $true
  $info.RedirectStandardError = $true
  $info.Arguments = '-c "import sys,tiktoken; print(len(tiktoken.get_encoding(''o200k_base'').encode(sys.stdin.read())))"'
  $process = [System.Diagnostics.Process]::new()
  $process.StartInfo = $info
  if (-not $process.Start()) { throw 'Could not start tokenizer' }
  $process.StandardInput.Write($Text)
  $process.StandardInput.Close()
  $count = $process.StandardOutput.ReadToEnd().Trim()
  $error = $process.StandardError.ReadToEnd()
  $process.WaitForExit()
  if ($process.ExitCode -ne 0) { throw "tokenizer failed: $error" }
  return [int]$count
}

function Measure-TokenOutput([string]$Name, [string[]]$Arguments) {
  $text = Invoke-Text -Arguments $Arguments
  [pscustomobject]@{
    name = $Name
    bytes_utf8 = [Text.Encoding]::UTF8.GetByteCount($text)
    chars = $text.Length
    tokens_o200k_base = Get-TokenCount -Text $text
    output = $text
  }
}

try {
  $results = @()
  $results += Measure-TokenOutput -Name 'stable-json-full-snapshot' -Arguments @('-s', $Serial, '-j', 'ui', 'snap')
  $results += Measure-TokenOutput -Name 'compact-snapshot' -Arguments @('-s', $Serial, '-c', 'ui', 'snap', '--compact')
  $results += Measure-TokenOutput -Name 'compact-frontier-snapshot' -Arguments @('-s', $Serial, '-c', 'ui', 'snap', '--compact', '--frontier')
  [void](Invoke-Text -Arguments @('-s', $Serial, '-c', 'ui', 'snap', '--compact', '--delta'))
  $results += Measure-TokenOutput -Name 'compact-stable-delta' -Arguments @('-s', $Serial, '-c', 'ui', 'snap', '--compact', '--delta')
  $results += Measure-TokenOutput -Name 'compact-find' -Arguments @('-s', $Serial, '-c', 'ui', 'find', $Selector)
  $results += Measure-TokenOutput -Name 'compact-proof' -Arguments @('-s', $Serial, '-c', 'exp', 'f1', $Selector, $Postcondition, '5000')
  $batch = "ui tap '$Selector'; ui wait '$Postcondition' 5000"
  $results += Measure-TokenOutput -Name 'compact-batch-proof' -Arguments @('-s', $Serial, '-c', 'b', $batch)
  $tape = "D0 '$Selector'; P '$Selector' '$Postcondition' 5000"
  $results += Measure-TokenOutput -Name 'compact-tape-proof' -Arguments @('-s', $Serial, '-c', 'x', $tape)
  $report = [pscustomobject]@{
    generated_at = (Get-Date).ToUniversalTime().ToString('o')
    serial = $Serial
    tokenizer = 'tiktoken o200k_base (measurement proxy; GPT-5.6 Luna tokenizer is not exposed locally)'
    results = $results | ForEach-Object {
      $_ | Select-Object name,bytes_utf8,chars,tokens_o200k_base
    }
    outputs = $results | ForEach-Object {
      [pscustomobject]@{ name = $_.name; output = $_.output }
    }
  }
  $path = Join-Path $OutputRoot 'report.json'
  $report | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $path -Encoding UTF8
  $report | ConvertTo-Json -Depth 8
  Write-Output "report=$path"
} finally {
  $env:PYTHONPATH = $oldPythonPath
}
