# Benchmark methodology

Benchmarks measure the agent interface, not just raw ADB. Each case records task definition, skilled-human estimate, agent completion time, command count, tool calls, input/output tokens where available, screenshots, retries, failures, recovery, transport, warmup, and sample count.

Recommended lanes:

- `B1` cold help and device discovery;
- `B2` open an app and verify the foreground window;
- `B3` find, edit, and submit text through accessibility handles;
- `B4` a settings workflow with one scroll and one dynamic state change;
- `B5` extract visible text from a deterministic page without an image;
- `B6` recover from a dialog or unexpected screen;
- `B7` twenty shell-compatible actions in one batch;
- `B8` one task that requires a screenshot because semantic completeness is false;
- `B9` camera/microphone finite capture and cleanup;
- `B10` mock location set, verify, route cancel, and restoration.

The primary comparisons are cold CLI p95, named-pipe ping p95, persistent no-op p95, shell batch completion, semantic snapshot/action p95, first scrcpy frame, binary size, and daemon idle working set. A human baseline is measured with a skilled operator using the same device and task fixture, excluding unrelated setup time.

Token efficiency is measured from compact output bytes/tokens, number of unchanged nodes returned, screenshot count, and recovery turns. A good run makes one observation, reuses handles, batches deterministic actions, and requests pixels only when the semantic completeness/confidence signal says text is insufficient.

## Release microbenchmarks

The retained release run used 30 samples after 5 warmups per lane. Persistent
measurements exclude process startup; `p95` is the gate value.

| Metric | USB | mDNS | Direct Wi-Fi | Gate | Result |
| --- | ---: | ---: | ---: | ---: | --- |
| Cold `au help` p95 | 11.154 ms | 8.980 ms | 10.418 ms | <=15 ms | pass |
| Named-pipe daemon ping p95 | 1.514 ms | 6.405 ms | 1.575 ms | <=5 ms | USB/IP pass; mDNS fail |
| Persistent remote no-op p95 | 19.665 ms | 53.179 ms | 54.159 ms | <=20/35 ms | USB pass; Wi-Fi fail |
| Twenty-action batch proof | `ok 20` | `ok 20` | `ok 20` | one transaction | pass |
| Idle daemon working set | ~7.2 MB | ~7.3 MB | — | <=25 MB | pass |
| Stripped host binary | 1,449,984 bytes | same build | same build | <=5 MB | pass |

The Wi-Fi failure is intentionally visible: identity-safe failover works, but
the current wireless endpoint does not meet the latency gate. The release is
therefore a prerelease, not a stable-performance claim.

A separate 100-sample wireless stress lane retained a 130.185 ms p95. It is
kept as corroborating tail-latency evidence rather than replaced by the more
favorable 30-sample release run.

## Agent task suite

The task-level suite is designed for a skilled-human comparison without
hardcoded coordinates: app launch, semantic selection, text editing, settings
scroll/recovery, deterministic web extraction, batched repetition, selective
vision, media cleanup, and mock-location restoration. Human duration and exact
GPT-token counts are run-specific and are not fabricated from byte counts; this
release records the protocol and host microbenchmarks. The retained held-out
Luna run covered 18 ordinary tasks (577,190 ms total; 17,793 ms median), but its
thread API did not expose exact model/tool/image tokens. Five independent
skilled-human comparison trials remain open; no human-parity claim is made.

## Contract episode baseline

Run `scripts/bench-agent-contract.ps1` on an authorized device to measure one
persistent JSONL server after warmup. It records status, object choices, dense
choices, and query response bytes plus mean/p50/p95 latency. The retained USB
run produced:

| Lane | p50 | Mean | Response bytes |
| --- | ---: | ---: | ---: |
| status | 313.889 ms | 555.716 ms | varies |
| object choices | 15.622 ms | 17.167 ms | 2,822 |
| dense choices | 12.694 ms | 17.078 ms | 498 |
| targeted query | 38.042 ms | 41.393 ms | varies |

Dense choices were 82.35% smaller than equivalent object choices and 88.73%
smaller than the original 4,418-byte rich response. Byte counts are reported
as bytes, not fabricated model-token counts; `token_count` remains null unless
a real tokenizer is explicitly selected.

## Long agentic device benchmark

`scripts/bench-agentic-device.ps1` is a guarded destructive benchmark. It
requires the exact enrolled identity plus explicit app-lifecycle and download
cleanup switches. It refuses to overwrite a pre-existing fixture package or
named download. The run installs only the zero-permission
`dev.codex.aubench` fixture, creates one disposable Chrome tab and one named
download, exercises native UI and browser UI through persistent sessions, and
then removes those exact resources.

```powershell
.\scripts\bench-agentic-device.ps1 `
  -Serial EXACT_ENROLLED_IDENTITY `
  -Minutes 15 `
  -AllowAppLifecycle `
  -AllowDownloadCleanup
```

The retained 15-minute USB run completed 290 native app episodes, 97 Chrome
episodes, 3,202 warm contract/pipe calls, 1,450 committed mutations, and 967
verified outcomes. Five generation races recovered automatically; no episode
failed. The mean dense response was 965.86 bytes across 1,430 observations.
Independent checks confirmed fixture uninstall, named-download removal,
disposable-tab closure, reverse removal, and final semantic readiness.

The release binary was also exercised in two independent 60-minute runs on the
same enrolled USB tablet. The first run completed its workload and cleanup but
failed the acceptance gate with one late `E_AUTH` after the helper's one-hour
session TTL expired. That was a real reliability failure, not a benchmark
exception. The host now treats that response as a pre-dispatch authentication
rejection, verifies device identity, opens one fresh helper session, and
retries the exact operation once; state-changing operations are still never
replayed after an ambiguous transport failure. The second 60-minute run then
passed with zero errors, zero transport disconnects, 1,425 native episodes,
476 browser episodes, 9,434 contract calls, 7,125 committed operations, 4,950
verified outcomes, 98 stale recoveries, and 948.62 mean dense-response bytes.
All fixture, download, tab, forward, rotation, and foreground cleanup checks
passed. The raw run reports remain private; the public aggregate is the
authoritative result.

The source-level comparison and the requirements for any public competitor
benchmark are documented in
[`competitive-landscape.md`](competitive-landscape.md). No cross-tool latency
or token superiority claim is made without that reproduced workload.

The long run is a reliability gate, not a claim that every Android task is
solved. It exercises the AU-owned fixture, Chrome, system UI, rotation, stale
state, helper/daemon/browser restarts, downloads, and cleanup. It does not
prove API-26 hardware behavior, all OEM accessibility implementations, or
unfamiliar app semantics; those remain partial or unsupported until separately
verified.

## Live unfamiliar-task evidence

The real tablet work deliberately includes tasks that were not prewritten as
AU fixtures. A Clash of Clans tutorial attack reached the authoritative victory
screen with 100% damage and three stars without spending gems; a reversible
display-timeout change was set, read back, restored, and read back again; and a
public YouTube lo-fi page was recorded as failed because no playing/advancing
state could be proven. Fresh Luna agents then completed independent browser,
download, and cleanup goals at low, medium, and max reasoning. The important
measurement is not just elapsed time: each result includes proof, retries,
stale-state recovery, and cleanup status.
