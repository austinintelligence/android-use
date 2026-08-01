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
