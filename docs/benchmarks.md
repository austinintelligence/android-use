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
