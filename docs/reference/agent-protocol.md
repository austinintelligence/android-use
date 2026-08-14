# Agent protocol

MCP exposes two tools. JSONL accepts the same request objects and returns one bounded JSON response per line.

## `android.read`

Required field: `q`.

| `q` | Optional fields | Result |
| --- | --- | --- |
| `status` | — | Readiness, generation, capability mask. |
| `observe` | `base`, `detail` | Semantic frontier or bounded delta. |
| `browser` | `op=tabs|observe|text` | Chrome state. |
| `capabilities` | — | Optional capability and permission state. |
| `location` | — | Current bounded location response. |
| `notifications` | — | Compact notification list. |
| `visual` | `op=hash|diff`, `a`, `b` | Bounded PNG metrics. |
| `artifact` | `id`, optional `range` | Base64 artifact bytes for one bounded range. |

## `android.act`

Every plan includes:

```json
{"id":"unique-operation-id","g":42,"p":[["tap",7],["wait",["text","Done"],3000]]}
```

- `id` is unique and stable for that intended operation.
- `g` is the generation returned by the relevant observation.
- `p` contains 1–32 forward-only operations and at most 16 mutations.
- `deadline_ms` may be 1–30000.
- `max_mutations` can lower the mutation ceiling.

Android operations include `tap`, `long`, `text`, `scroll`, `key`, `gesture`, `launch`, `wait`, `assert`, screen/camera/microphone/screen-record capture, and notification actions.

A browser plan adds `"target":"browser"` and uses browser generation. It supports `navigate`, `back`, `forward`, `reload`, `click`, `focus`, `text`, `key`, `scroll`, `wait`, bounded `eval`, `screenshot`, `select`, `close`, and `new`.

A visual plan adds `"target":"visual"` and performs one bounded crop of a host PNG artifact.

## Outcomes

`ok:1` includes the resulting generation and mutation count. `stale` means no mutation began. `partial` means at least one mutation occurred before failure. `unknown` means the host cannot prove the outcome. Read current state before any recovery action.

For the exact JSON Schema, inspect the tool descriptors returned by MCP initialization. The implementation in `computer/src/api.rs` is the canonical source contract.
