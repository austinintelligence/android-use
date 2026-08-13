[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$skillRoot = Join-Path $root 'skills\android-use'
$skillPath = Join-Path $skillRoot 'SKILL.md'
$schemaPath = Join-Path $skillRoot 'references\protocol-schema.json'
$tapePath = Join-Path $root 'crates\android-use\src\tape.rs'

foreach ($required in @($skillPath, (Join-Path $skillRoot 'agents\openai.yaml'), $schemaPath)) {
  if (-not (Test-Path -LiteralPath $required -PathType Leaf)) {
    throw "skill file is missing: $required"
  }
}

$skill = Get-Content -LiteralPath $skillPath -Raw
if ($skill -notmatch '(?s)^---\nname: android-use\ndescription: .+?\n---\n') {
  throw 'SKILL.md must begin with name and description frontmatter only'
}
if ((Get-Content -LiteralPath $skillPath).Count -gt 500) {
  throw 'SKILL.md exceeds the 500-line context budget'
}

$linked = [regex]::Matches($skill, '\]\((references/[^)]+)\)') |
  ForEach-Object { $_.Groups[1].Value } |
  Sort-Object -Unique
foreach ($relative in $linked) {
  $target = Join-Path $skillRoot $relative
  if (-not (Test-Path -LiteralPath $target -PathType Leaf)) {
    throw "SKILL.md reference is missing: $relative"
  }
}

$schema = Get-Content -LiteralPath $schemaPath -Raw | ConvertFrom-Json
$tapeSource = Get-Content -LiteralPath $tapePath -Raw
$opcodeMatch = [regex]::Match($tapeSource, 'pub const OPCODES: &\[char\] = &\[(?<body>.*?)\];', [Text.RegularExpressions.RegexOptions]::Singleline)
if (-not $opcodeMatch.Success) { throw 'tape opcode table is missing from tape.rs' }
$sourceOpcodes = @([regex]::Matches($opcodeMatch.Groups['body'].Value, "'([A-Z])'") | ForEach-Object { $_.Groups[1].Value })
$schemaOpcodes = @($schema.opcodes.psobject.Properties | ForEach-Object { $_.Name.ToUpperInvariant() })
if ((@($sourceOpcodes | Sort-Object) -join ',') -ne (@($schemaOpcodes | Sort-Object) -join ',')) {
  throw "protocol schema opcode drift: Rust=[$($sourceOpcodes -join ',')] schema=[$($schemaOpcodes -join ',')]"
}

Write-Output "skill passed files=$($linked.Count + 3) lines=$((Get-Content -LiteralPath $skillPath).Count)"
