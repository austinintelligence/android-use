[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$schemaPath = Join-Path $root 'skills\android-use\references\agent-contract.json'
$rustOutput = & cargo run --quiet --package android-use --bin au -- schema --json
if ($LASTEXITCODE -ne 0) { throw 'au schema failed' }
$envelope = $rustOutput | ConvertFrom-Json
$schema = $envelope.data.data
$static = Get-Content -LiteralPath $schemaPath -Raw | ConvertFrom-Json

if ([int]$schema.properties.v.const -ne 2) { throw 'Rust schema is not contract v2' }
$methods = @($schema.properties.method.enum)
$expected = @('android.status', 'android.observe', 'android.execute', 'android.artifact', 'android.recipe')
if ((Compare-Object $methods $expected)) { throw 'Rust method list drifted from the canonical contract' }
if ([int]$static.version -ne 2) { throw 'static agent contract is not v2' }
if ((Compare-Object @($static.methods) $expected)) { throw 'static agent contract method list drifted' }
Write-Output 'agent contract schema is synchronized'
