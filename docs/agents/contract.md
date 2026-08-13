# Agent contract

This is the stable agent boundary for Android Use. It is available over `au serve --jsonl`, `au serve --mcp`, and the compatibility CLI commands `au observe`, `au execute`, `au artifact`, and `au recipe`.

The canonical methods are:

- `android.status` — exact device identity, transport, helper, capability, and readiness state.
- `android.observe` — bounded, redacted semantic evidence.
- `android.execute` — bounded semantic plans with generation checks and receipts.
- `android.artifact` — bounded access to AU-owned artifact handles.
- `android.recipe` — validated declarative workflows.

Every request includes a schema identifier and request ID:

```json
{"v":2,"id":"r1","method":"android.observe","params":{"mode":"choices","encoding":"dense"}}
```

When a device is specified, `device.serial` is the exact enrolled
`ro.serialno` hardware identity and `device.endpoint` is the reachable ADB
endpoint (USB serial, `host:port`, or mDNS selector). Keep both when a caller
needs to pin a particular transport. A serial-only request uses AU's
identity-checked endpoint selection; a remote-only `device.remote_id` is
rejected until the encrypted remote broker exists.

The safe execution surface contains only semantic operations: `find`, `tap`, `long`, `set`, `scroll`, `global`, `wait`, `assert`, `observe`, and `launch`. Raw ADB, raw shell, arbitrary code, unrestricted paths, and unbounded loops remain compatibility-only operations and are not exposed through the contract.

## Dense observations

`encoding: "object"` returns named choice objects for debugging and adapters. `encoding: "dense"` returns the model-native envelope `{"v":2,"o":OBS,"g":GEN,"m":"choices","d":{"n":[...]}}`. Each `d.n` row is `[stable_ref,label,role,flags]`; flags are `1=clickable`, `2=enabled`, `4=checked`, `8=scrollable`, `16=visible`, and `32=redacted`. Empty or irrelevant metadata is omitted. The caller can request `query`, `context`, or `expanded` mode only when choices do not answer the next decision.

The server lifetime owns warm device-selection, shell, helper, and browser pools. Capability data has a two-second cache, while reachability and exact identity remain live checks. Pass `fresh: true` to `android.status` when a capability mutation must bypass that cache.

`android.status` parameters are strict: `device` optionally pins the enrolled
identity/endpoint and `fresh` bypasses the short capability cache. Unknown
fields are rejected instead of being ignored.

## Execution and receipts

`android.execute` requires a bounded deadline, a maximum mutation count, and optional identity/generation preconditions. Supported adjacent semantic steps compile to one validated device-resident `plan.run` frame; the same executor is used by compact batches. The common tap/wait/assert shape retains the smaller `helper-proof` fast path. App launches and operations not yet supported by `plan.run` stay at the host boundary.

Receipts distinguish three outcomes:

- `committed`: every planned step and postcondition completed.
- `E_PARTIAL`: the helper returned the exact completed prefix and committed mutation count; observe and re-plan from current state.
- `E_UNKNOWN_COMMIT`: transport failed after a mutation may have been delivered; observe before any retry and never replay blindly.

Stable references are session-scoped. `E_STALE` means the generation changed before dispatch; make one fresh observation and rebuild the plan. A client may retry generation races with a new operation ID, but must not reuse an operation ID that has a partial or unknown receipt.

An observation that exceeds its byte budget returns `E_OUTPUT_LIMIT` with its
measured size, budget, mode, and a next-step hint; AU does not return a
successful-looking truncated observation.

The machine-readable source is [`agent-contract.json`](../../skills/android-use/references/agent-contract.json). The Rust implementation is [`contract.rs`](../../crates/android-use/src/contract.rs).
