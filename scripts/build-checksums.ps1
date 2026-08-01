[CmdletBinding()]
param(
  [Parameter(Mandatory = $true)][string]$Directory,
  [string]$Output
)

$ErrorActionPreference = 'Stop'
$Directory = (Resolve-Path -LiteralPath $Directory).Path
if ([string]::IsNullOrWhiteSpace($Output)) {
  $Output = Join-Path $Directory 'checksums.txt'
}
$rows = foreach ($file in (Get-ChildItem -LiteralPath $Directory -File | Sort-Object Name)) {
  if ($file.FullName -eq (Resolve-Path -LiteralPath $Output -ErrorAction SilentlyContinue).Path) { continue }
  $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
  "$hash *$($file.Name)"
}
$utf8 = [System.Text.UTF8Encoding]::new($false)
[IO.File]::WriteAllLines($Output, @($rows), $utf8)
Write-Output "checksums=$Output files=$(@($rows).Count)"
