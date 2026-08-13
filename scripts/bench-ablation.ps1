[CmdletBinding()]
param(
  [string]$Root,
  [string]$OutputPath,
  [switch]$Force
)

$ErrorActionPreference = 'Stop'
$scriptRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
if ([string]::IsNullOrWhiteSpace($Root)) { $Root = Split-Path -Parent $scriptRoot }
if ([string]::IsNullOrWhiteSpace($OutputPath)) {
  $OutputPath = Join-Path $Root 'artifacts\final\ablation-matrix.json'
}
if ((Test-Path -LiteralPath $OutputPath) -and -not $Force) {
  throw "output exists; pass -Force to replace it: $OutputPath"
}
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $OutputPath) | Out-Null

function Read-Report([string]$RelativePath) {
  $path = Join-Path $Root $RelativePath
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { return $null }
  return (Get-Content -LiteralPath $path -Raw | ConvertFrom-Json)
}

function Find-Metric($Report, [string]$Name, [string]$Field = 'p95_ms') {
  if ($null -eq $Report) { return $null }
  $metric = @($Report.metrics | Where-Object { $_.name -eq $Name }) | Select-Object -First 1
  if ($null -eq $metric) { return $null }
  return $metric.$Field
}

$usb = Read-Report 'artifacts\benchmarks\usb-vision-final\report.json'
$wifi = Read-Report 'artifacts\benchmarks\wifi-vision-final\report.json'
$usbNativePath = if (Test-Path (Join-Path $Root 'artifacts\benchmarks\usb-native-post-boundary\report.json')) { 'artifacts\benchmarks\usb-native-post-boundary\report.json' } elseif (Test-Path (Join-Path $Root 'artifacts\benchmarks\usb-native-current\report.json')) { 'artifacts\benchmarks\usb-native-current\report.json' } else { 'artifacts\benchmarks\usb-native-rust-r2\report.json' }
$wifiNativePath = if (Test-Path (Join-Path $Root 'artifacts\benchmarks\wifi-native-post-boundary\report.json')) { 'artifacts\benchmarks\wifi-native-post-boundary\report.json' } elseif (Test-Path (Join-Path $Root 'artifacts\benchmarks\wifi-native-current\report.json')) { 'artifacts\benchmarks\wifi-native-current\report.json' } else { 'artifacts\benchmarks\wifi-native-rust\report.json' }
$usbNative = Read-Report $usbNativePath
$wifiNative = Read-Report $wifiNativePath
$codec = Read-Report 'artifacts\final\codec-evaluation.json'
$usbNoopP95 = if ($null -ne $usbNative) { $usbNative.data.p95_ms } else { Find-Metric $usb 'persistent_batch_noop' }
$wifiNoopP95 = if ($null -ne $wifiNative) { $wifiNative.data.p95_ms } else { Find-Metric $wifi 'persistent_batch_noop' }
$rows = @(
  [pscustomobject]@{ id='E0'; variable='Frozen current baseline'; status='MEASURED'; correctness='PASS'; first_pass='not model-measured'; usb_p95_ms=(Find-Metric $usb 'direct_status_no_daemon'); wifi_p95_ms=(Find-Metric $wifi 'direct_status_no_daemon'); exact_tokens='UNAVAILABLE'; evidence=@('artifacts/benchmarks/usb-final/report.json','artifacts/benchmarks/wifi-final/report.json'); note='Historical baseline is preserved; model-visible metrics were not available.' },
  [pscustomobject]@{ id='E1'; variable='Execute-first daemon and config cache'; status='PARTIAL'; correctness='PASS'; first_pass='not isolated'; usb_p95_ms=$usbNoopP95; wifi_p95_ms=$wifiNoopP95; exact_tokens='UNAVAILABLE'; evidence=@($usbNativePath,$wifiNativePath,'crates/android-use/src/daemon.rs','crates/android-use/src/config.rs'); note='Persistent path is measured, but no clean E0/E1-only checkout comparison was retained.' },
  [pscustomobject]@{ id='E2'; variable='Cached device inventory and event tracking'; status='PARTIAL'; correctness='PASS'; usb_p95_ms=$null; wifi_p95_ms=$null; exact_tokens='UNAVAILABLE'; evidence=@('crates/android-use/src/device.rs','skills/android-use/references/device-selection.md','artifacts/final/evidence-matrix.md'); note='Exact-serial selection/failover and event-aware helper state are tested; isolated latency attribution is unavailable.' },
  [pscustomobject]@{ id='E3'; variable='Persistent helper forward and socket'; status='PARTIAL'; correctness='PASS'; usb_p95_ms=$null; wifi_p95_ms=$null; exact_tokens='UNAVAILABLE'; evidence=@('crates/android-use/src/helper.rs','skills/android-use/references/helper-install.md','artifacts/final/evidence-matrix.md'); note='Live restart/recovery passed on both transports; no isolated model-cost run was captured.' },
  [pscustomobject]@{ id='E4'; variable='Persistent CDP forward and WebSocket'; status='PARTIAL'; correctness='PASS'; usb_p95_ms=$null; wifi_p95_ms=$null; exact_tokens='UNAVAILABLE'; evidence=@('crates/android-use/src/web.rs','skills/android-use/references/web-cdp.md','artifacts/final/evidence-matrix.md'); note='CDP reuse and cleanup passed live; no isolated A/B timing ledger exists.' },
  [pscustomobject]@{ id='E5'; variable='Lazy evidence construction'; status='MEASURED'; correctness='PASS'; usb_p95_ms=$null; wifi_p95_ms=$null; exact_tokens='PROXY_ONLY'; evidence=@('artifacts/final/codec-evaluation.json','artifacts/token-bench'); note='Full, compact, frontier, delta, proof, and batch payloads were measured with the labeled o200k proxy.' },
  [pscustomobject]@{ id='E6'; variable='Semantic frontier compiler'; status='MEASURED'; correctness='PASS'; usb_p95_ms=$null; wifi_p95_ms=$null; exact_tokens='PROXY_ONLY'; evidence=@('skills/android-use/references/semantic-ui.md','artifacts/final/evidence-matrix.md','artifacts/token-bench'); note='Complete frontier and delta behavior passed USB/Wi-Fi; semantic reduction is measured, not an agent-comprehension proof.' },
  [pscustomobject]@{ id='E7'; variable='Proof-carrying helper VM'; status='MEASURED'; correctness='PASS'; usb_p95_ms=$usbNoopP95; wifi_p95_ms=$wifiNoopP95; exact_tokens='PROXY_ONLY'; evidence=@('crates/android-use/src/tape.rs','artifacts/falsification/f1-summary.md','artifacts/final/evidence-matrix.md'); note='Bounded tape and proof receipts pass unit/live checks; model turns and exact tokens remain unavailable.' },
  [pscustomobject]@{ id='E8'; variable='Adaptive event pacing'; status='MEASURED'; correctness='PASS'; usb_p95_ms=(Find-Metric $usb 'persistent_batch_20_zero_delay'); wifi_p95_ms=(Find-Metric $wifi 'persistent_batch_20_zero_delay'); exact_tokens='UNAVAILABLE'; evidence=@('crates/android-use/src/batch.rs','artifacts/benchmarks/usb-native-final/report.json','artifacts/benchmarks/wifi-native-final/report.json'); note='Default shell pacing is measured separately from explicit zero-delay transaction; semantic actions do not inherit the 250 ms delay.' },
  [pscustomobject]@{ id='E9'; variable='Token-tape vocabulary'; status='MEASURED'; correctness='PASS'; usb_p95_ms=$null; wifi_p95_ms=$null; exact_tokens='PROXY_ONLY'; evidence=@('crates/android-use/src/tape.rs','skills/android-use/references/tape-protocol.md','artifacts/final/codec-evaluation.json'); note='Five complete corpus cases compare compatibility JSON, compact JSON, and tape; exact Luna tokenizer is unavailable.' },
  [pscustomobject]@{ id='E10'; variable='Session dictionary'; status='MEASURED'; correctness='PASS'; usb_p95_ms=$null; wifi_p95_ms=$null; exact_tokens='PROXY_ONLY'; evidence=@('crates/android-use/src/tape.rs','skills/android-use/references/tape-protocol.md','artifacts/final/evidence-matrix.md'); note='Dictionary epoch/checksum/reset and daemon-session reuse passed; no exact model token ledger.' },
  [pscustomobject]@{ id='E11'; variable='Compact typed errors'; status='MEASURED'; correctness='PASS'; usb_p95_ms=$null; wifi_p95_ms=$null; exact_tokens='PROXY_ONLY'; evidence=@('crates/android-use/src/output.rs','crates/android-use/src/protocol.rs','artifacts/final/codec-evaluation.json'); note='Typed E_STALE/E_FRAME/E_PROTOCOL paths are tested and compact errors are corpus-measured.' },
  [pscustomobject]@{ id='E12'; variable='Adaptive response budgets'; status='PARTIAL'; correctness='PASS'; usb_p95_ms=$null; wifi_p95_ms=$null; exact_tokens='PROXY_ONLY'; evidence=@('crates/android-use/src/output.rs','crates/android-use/src/process.rs','artifacts/final/evidence-matrix.md'); note='Output caps, file redirection, compact proof, and binary gating are measured; adaptive model-budget impact is not isolated.' },
  [pscustomobject]@{ id='E13'; variable='Visual hashes and crops'; status='MEASURED'; correctness='PASS'; usb_p95_ms=$null; wifi_p95_ms=$null; exact_tokens='UNAVAILABLE'; evidence=@('crates/android-use/src/vision.rs','skills/android-use/references/vision.md','artifacts/final/evidence-matrix.md'); note='Semantic inspect, hash, diff, crop, region/check, stale rejection passed USB/Wi-Fi.' },
  [pscustomobject]@{ id='E14'; variable='Narrow direct ADB-server client'; status='REJECTED'; correctness='NOT_APPLICABLE'; usb_p95_ms=$null; wifi_p95_ms=$null; exact_tokens='UNAVAILABLE'; evidence=@('crates/android-use/src/adb.rs','artifacts/final/evidence-matrix.md'); note='Not implemented: the measured ADB/server path did not justify replacing the bounded official ADB subprocess boundary.' },
  [pscustomobject]@{ id='E15'; variable='Combined AU/2 candidate'; status='MEASURED'; correctness='PASS'; usb_p95_ms=$usbNoopP95; wifi_p95_ms=$wifiNoopP95; exact_tokens='PROXY_ONLY'; evidence=@($usbNativePath,$wifiNativePath,'artifacts/final/evidence-matrix.md'); note='Current-candidate native measurements are retained; USB is near the 20 ms gate and current Wi-Fi is noisy; full held-out agent suite is still open.' },
  [pscustomobject]@{ id='E16'; variable='Lean packaging and deployment'; status='MEASURED'; correctness='PASS'; usb_p95_ms=$null; wifi_p95_ms=$null; exact_tokens='UNAVAILABLE'; evidence=@('scripts/build-helper.ps1','scripts/validate-skill.ps1','artifacts/rollback','artifacts/final/rollback-validation-2026-08-01.md','artifacts/final/evidence-matrix.md'); note='Pinned toolchain, validated skill, hashes, signed APK, rollback copies, and a live installed-root candidate swap/restore are verified.' }
)

$report = [pscustomobject]@{
  generated_at = (Get-Date).ToUniversalTime().ToString('o')
  candidate_binary_sha256 = (Get-FileHash -LiteralPath (Join-Path $Root 'target\release\au.exe') -Algorithm SHA256).Hash.ToLowerInvariant()
  tokenizer = 'exact GPT-5.6 Luna tokenizer unavailable; PROXY_ONLY values use artifacts/final/codec-evaluation.json'
  required_fields = @('correctness','first_pass','median','p95','cold_penalty','warm_performance','model_turns','tool_calls','exact_tokens','process_count','adb_count','socket_count','bytes','retries','recovery','unintended_side_effects','cleanup','regression_status')
  experiments = $rows
  conclusion = 'The combined candidate is retained. Partial rows identify non-isolated historical evidence; no model-efficiency or human-parity gate is claimed.'
}
$utf8 = [System.Text.UTF8Encoding]::new($false)
[IO.File]::WriteAllText($OutputPath, ($report | ConvertTo-Json -Depth 12), $utf8)
$report | ConvertTo-Json -Depth 12
Write-Output "report=$OutputPath"
