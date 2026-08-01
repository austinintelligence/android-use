# Bounded trace ledger

Tracing is opt-in and outside the normal model hot path:

```text
au -w --trace PATH st
au -w --trace PATH b "home;back"
```

`--trace PATH` appends bounded JSONL events. The client generates one safe
trace ID and propagates it to the daemon as a hidden `--trace-id` operand.
Every event carries `v`, `id`, process id `pid`, monotonic microseconds `us`,
and sequence `q`; `p` is the phase. Event lines are capped at 16 KiB and
oversized fields collapse to a `trace.event_truncated` record. Trace I/O is
best-effort after the path has been opened, so diagnostics cannot change task
correctness.

The current phases include:

| Phase | Meaning |
|---|---|
| `cli.process`, `cli.dispatch` | parser/process boundary and daemon decision |
| `daemon.client_execute`, `daemon.request`, `daemon.response` | named-pipe request lifecycle |
| `action.execute`, `action.daemon_execute`, `action.result` | typed action dispatch and proof/error |
| `device.discover`, `device.resolve` | endpoint inventory and exact-identity selection |
| `child.run` | bounded ADB/helper child timing, without command arguments |
| `helper.open`, `helper.call` | authenticated helper session and operation timing |
| `web.execute` | CDP/web operation timing |
| `output.success`, `output.error` | final response boundary |

Trace records never include raw command arguments, helper tokens, page text,
media bytes, or captured private content. Use a unique artifact path per run;
the writer is append-only and never silently overwrites an existing report.
