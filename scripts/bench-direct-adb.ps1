[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Before,
    [Parameter(Mandatory = $true)][string]$After,
    [Parameter(Mandatory = $true)][string]$Output,
    [ValidateRange(10, 1000)][int]$Samples = 80,
    [ValidateRange(1, 100)][int]$Warmup = 8
)

$ErrorActionPreference = 'Stop'
$beforePath = (Resolve-Path -LiteralPath $Before).Path
$afterPath = (Resolve-Path -LiteralPath $After).Path
$outputPath = [IO.Path]::GetFullPath($Output)
if (Test-Path -LiteralPath $outputPath) {
    throw "Output already exists: $outputPath"
}

function Invoke-Sample([string]$Binary) {
    $started = [Diagnostics.Stopwatch]::GetTimestamp()
    $result = & $Binary --no-daemon st 2>&1
    $exitCode = $LASTEXITCODE
    $elapsed = ([Diagnostics.Stopwatch]::GetTimestamp() - $started) * 1000.0 / [Diagnostics.Stopwatch]::Frequency
    if ($exitCode -ne 0) {
        throw "Status benchmark failed with exit code $exitCode"
    }
    [pscustomobject]@{
        ms = [math]::Round($elapsed, 4)
        output_bytes = [Text.Encoding]::UTF8.GetByteCount(($result | Out-String))
    }
}

for ($index = 0; $index -lt $Warmup; $index++) {
    $null = Invoke-Sample $beforePath
    $null = Invoke-Sample $afterPath
}

$beforeSamples = [Collections.Generic.List[object]]::new()
$afterSamples = [Collections.Generic.List[object]]::new()
for ($index = 0; $index -lt $Samples; $index++) {
    if (($index % 2) -eq 0) {
        $beforeSamples.Add((Invoke-Sample $beforePath))
        $afterSamples.Add((Invoke-Sample $afterPath))
    } else {
        $afterSamples.Add((Invoke-Sample $afterPath))
        $beforeSamples.Add((Invoke-Sample $beforePath))
    }
}

function Get-Quantile([double[]]$Values, [double]$Quantile) {
    $sorted = @($Values | Sort-Object)
    $index = [math]::Max(0, [math]::Ceiling($Quantile * $sorted.Count) - 1)
    [math]::Round($sorted[$index], 4)
}

function Get-Summary([Collections.Generic.List[object]]$Values) {
    [double[]]$times = @($Values | ForEach-Object { $_.ms })
    [pscustomobject]@{
        n = $times.Count
        p50_ms = Get-Quantile $times 0.50
        p95_ms = Get-Quantile $times 0.95
        p99_ms = Get-Quantile $times 0.99
        mean_ms = [math]::Round(($times | Measure-Object -Average).Average, 4)
        min_ms = [math]::Round(($times | Measure-Object -Minimum).Minimum, 4)
        max_ms = [math]::Round(($times | Measure-Object -Maximum).Maximum, 4)
        output_bytes = @($Values | ForEach-Object { $_.output_bytes } | Sort-Object -Unique)
    }
}

$beforeSummary = Get-Summary $beforeSamples
$afterSummary = Get-Summary $afterSamples
$report = [ordered]@{
    schema = 1
    benchmark = 'physical-usb-status-direct-adb-ablation'
    generated_utc = [DateTime]::UtcNow.ToString('o')
    samples = $Samples
    warmup = $Warmup
    ordering = 'alternating'
    command = 'au --no-daemon st'
    before = [ordered]@{
        sha256 = (Get-FileHash -LiteralPath $beforePath -Algorithm SHA256).Hash.ToLowerInvariant()
        bytes = (Get-Item -LiteralPath $beforePath).Length
        summary = $beforeSummary
        raw_ms = @($beforeSamples | ForEach-Object { $_.ms })
    }
    after = [ordered]@{
        sha256 = (Get-FileHash -LiteralPath $afterPath -Algorithm SHA256).Hash.ToLowerInvariant()
        bytes = (Get-Item -LiteralPath $afterPath).Length
        summary = $afterSummary
        raw_ms = @($afterSamples | ForEach-Object { $_.ms })
    }
    change = [ordered]@{
        p50_percent = [math]::Round((1.0 - $afterSummary.p50_ms / $beforeSummary.p50_ms) * 100.0, 2)
        p95_percent = [math]::Round((1.0 - $afterSummary.p95_ms / $beforeSummary.p95_ms) * 100.0, 2)
        p99_percent = [math]::Round((1.0 - $afterSummary.p99_ms / $beforeSummary.p99_ms) * 100.0, 2)
    }
}

$directory = Split-Path -Parent $outputPath
New-Item -ItemType Directory -Force -Path $directory | Out-Null
[IO.File]::WriteAllText($outputPath, (($report | ConvertTo-Json -Depth 8) + [Environment]::NewLine), [Text.UTF8Encoding]::new($false))
$report | ConvertTo-Json -Depth 6 -Compress
