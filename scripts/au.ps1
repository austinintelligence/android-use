$ErrorActionPreference = 'Stop'

$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
$auExecutable = Join-Path $scriptRoot 'rust\target\release\au.exe'
if (-not (Test-Path -LiteralPath $auExecutable -PathType Leaf)) {
    Write-Output 'err E_BUILD au.exe is not built; run scripts\build-au.cmd'
    exit 2
}

# Do not interpolate or reparse caller values. Windows PowerShell 5.1 does not
# expose ProcessStartInfo.ArgumentList, so build the documented Windows argv
# quoting form for ProcessStartInfo.Arguments. This keeps embedded quotes,
# angle brackets, dollar signs, and JavaScript source in one argv entry.
function ConvertTo-WindowsArgument([string] $argument) {
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

$processInfo = [System.Diagnostics.ProcessStartInfo]::new()
$processInfo.FileName = $auExecutable
$processInfo.UseShellExecute = $false
$processInfo.Arguments = (@($args) | ForEach-Object { ConvertTo-WindowsArgument ([string]$_) }) -join ' '
$process = [System.Diagnostics.Process]::new()
$process.StartInfo = $processInfo
if (-not $process.Start()) {
    Write-Output 'err E_SPAWN could not start au.exe'
    exit 2
}
$process.WaitForExit()
exit $process.ExitCode
