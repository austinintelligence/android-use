[CmdletBinding()]
param(
  [string]$Root,
  [string]$OutputPath,
  [switch]$Force
)

$ErrorActionPreference = 'Stop'
$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
if ([string]::IsNullOrWhiteSpace($Root)) { $Root = Split-Path -Parent $scriptRoot }
$tokenizerRoot = Join-Path $Root 'artifacts\tools\tiktoken'
if (-not (Test-Path -LiteralPath $tokenizerRoot -PathType Container)) {
  throw "tiktoken measurement environment is missing: $tokenizerRoot"
}
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
  $OutputPath = Join-Path $Root 'artifacts\final\codec-evaluation.json'
}
if ((Test-Path -LiteralPath $OutputPath) -and -not $Force) {
  throw "output exists; pass -Force to replace it: $OutputPath"
}
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $OutputPath) | Out-Null

$oldPythonPath = $env:PYTHONPATH
$env:PYTHONPATH = $tokenizerRoot

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

function Json([object]$Value) {
  return ($Value | ConvertTo-Json -Compress -Depth 12)
}

function Measure-Text([string]$Text) {
  [pscustomobject]@{
    bytes_utf8 = [Text.Encoding]::UTF8.GetByteCount($Text)
    tokens_o200k_base = Get-TokenCount -Text $Text
  }
}

$unicode = "line one`nline two: caf" + [char]0x00e9 + " " + [char]0x2014 + " " + ([char]0x3053 + [char]0x3093 + [char]0x306b + [char]0x3061 + [char]0x306f) + " " + [char]::ConvertFromUtf32(0x1f680) + "; quote=' " + '"' + " ; shell=" + '`$(id)'
$cases = @(
  [pscustomobject]@{
    name = 'find-proof'
    request = @{tool='android-use';command='ui';args=@('find','text~Allow,clickable=true#0');state=@{v=1;g=44;complete=$true}}
    compact = @{q='f';a=@('text~Allow,clickable=true#0');s=@{g=44}}
    tape = "D0 'text~Allow,clickable=true#0'; P @0 'text~Done' 3000"
    response = @{o=1;d=@{id='n7';proof='find.unique>tap>wait>assert'}}
    compact_response = '{"o":1,"p":"n7"}'
    tape_response = '{"o":1,"e":3,"h":"4f9a20b1","p":"n7"}'
  },
  [pscustomobject]@{
    name = 'dynamic-frontier'
    request = @{tool='android-use';command='ui';args=@('snap','--frontier');state=@{v=1;g=45;frontier=$true}}
    compact = @{q='q';a=@('f');s=@{g=45}}
    tape = 'Q'
    response = @{o=1;d=@{v=1;g=45;complete=$true;n=@(@(7,'POST DETERMINISTIC NOTIFICATION','','button',3,@(40,200,500,280)))}}
    compact_response = '{"o":1,"g":45,"n":[[7,"POST DETERMINISTIC NOTIFICATION","", "b",3,[40,200,500,280]]]}'
    tape_response = '{"o":1,"e":3,"h":"4f9a20b1","p":{"g":45,"n":[[7,"POST DETERMINISTIC NOTIFICATION"]]}}'
  },
  [pscustomobject]@{
    name = 'error-stale'
    request = @{tool='android-use';command='ui';args=@('tap','$0');state=@{v=1;g=45}}
    compact = @{q='t';a=@('$0');s=@{g=45}}
    tape = 'T $0'
    response = @{o=0;e='E_STALE';m='stale node handle; refresh the UI snapshot'}
    compact_response = '{"o":0,"e":"E_STALE","m":"refresh Q/F0"}'
    tape_response = '{"o":0,"e":"E_STALE","m":"Q;F0"}'
  },
  [pscustomobject]@{
    name = 'arbitrary-unicode-text'
    request = @{tool='android-use';command='ui';args=@('set','$0',$unicode);state=@{v=1;g=45}}
    compact = @{q='e';a=@('$0',$unicode);s=@{g=45}}
    tape = "E `$0 '$unicode'"
    response = @{o=1;d=@{text=$unicode}}
    compact_response = '{"o":1,"t":"'+$unicode+'"}'
    tape_response = '{"o":1,"t":"ok"}'
  },
  [pscustomobject]@{
    name = 'batch-recovery'
    request = @{tool='android-use';command='b';args=@("if ui:text~Ready then tx '$unicode'; wait ui:text~Done 3000");state=@{v=1;g=45}}
    compact = @{q='b';a=@("if ui:text~Ready then tx '$unicode'; wait ui:text~Done 3000");s=@{g=45}}
    tape = "D0 'text~Ready'; D1 '$unicode'; P @0 'text~Done' 3000"
    response = @{o=1;n=2;d=@{persistent_transaction=$true}}
    compact_response = '{"o":1,"n":2}'
    tape_response = '{"o":1,"n":2,"e":4,"h":"af8102dd"}'
  }
)

try {
  $rows = foreach ($case in $cases) {
    $compat = Json @{tool='android-use';request=$case.request;response=$case.response}
    $compact = Json @{q=$case.compact.q;a=$case.compact.a;s=$case.compact.s;r=$case.compact_response}
    $tape = "x $($case.tape)`n$($case.tape_response)"
    $word = "android-use $($case.name) => ok $($case.compact_response)"
    $compatMeasured = Measure-Text $compat
    $compactMeasured = Measure-Text $compact
    $tapeMeasured = Measure-Text $tape
    $wordMeasured = Measure-Text $word
    [pscustomobject]@{
      name = $case.name
      bytes = [pscustomobject]@{compatibility_json=$compatMeasured.bytes_utf8;compact_json=$compactMeasured.bytes_utf8;model_tape=$tapeMeasured.bytes_utf8;short_words=$wordMeasured.bytes_utf8}
      tokens_o200k_base = [pscustomobject]@{compatibility_json=$compatMeasured.tokens_o200k_base;compact_json=$compactMeasured.tokens_o200k_base;model_tape=$tapeMeasured.tokens_o200k_base;short_words=$wordMeasured.tokens_o200k_base}
    }
  }

  $skillPath = Join-Path $Root 'SKILL.md'
  $skillText = Get-Content -LiteralPath $skillPath -Raw
  $modelTapeTokens = @($rows | ForEach-Object { [int]$_.tokens_o200k_base.model_tape } | Sort-Object)
  $compactJsonTokens = @($rows | ForEach-Object { [int]$_.tokens_o200k_base.compact_json } | Sort-Object)
  $compatibilityJsonTokens = @($rows | ForEach-Object { [int]$_.tokens_o200k_base.compatibility_json } | Sort-Object)
  $middle = [Math]::Floor(($modelTapeTokens.Count - 1) / 2)
  $report = [pscustomobject]@{
    generated_at = (Get-Date).ToUniversalTime().ToString('o')
    tokenizer = 'tiktoken o200k_base (proxy; GPT-5.6 Luna tokenizer is not exposed locally)'
    corpus = @($cases | ForEach-Object { $_.name })
    skill = [pscustomobject]@{bytes_utf8=[Text.Encoding]::UTF8.GetByteCount($skillText);tokens_o200k_base=Get-TokenCount $skillText}
    cases = @($rows)
    summary = [pscustomobject]@{
      median_model_tape_tokens = $modelTapeTokens[$middle]
      median_compact_json_tokens = $compactJsonTokens[$middle]
      median_compatibility_json_tokens = $compatibilityJsonTokens[$middle]
    }
  }
  $utf8 = [System.Text.UTF8Encoding]::new($false)
  [IO.File]::WriteAllText($OutputPath, ($report | ConvertTo-Json -Depth 12), $utf8)
  $report | ConvertTo-Json -Depth 12
  Write-Output "report=$OutputPath"
} finally {
  $env:PYTHONPATH = $oldPythonPath
}
