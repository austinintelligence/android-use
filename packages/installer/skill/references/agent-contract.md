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

`android.status` accepts an optional `device` selector and `fresh: true` to
bypass the short capability cache after a capability change. Unknown fields
are rejected so a typo cannot silently select a stale default.

`device.serial` means the exact enrolled `ro.serialno` hardware identity;
`device.endpoint` means the reachable ADB endpoint. A serial-only request
uses identity-checked endpoint selection, while `device.remote_id` remains
fail-closed until the encrypted remote broker is implemented.

The safe execution surface contains only semantic operations: `find`, `tap`, `long`, `set`, `scroll`, `global`, `wait`, `assert`, `observe`, and `launch`. Raw ADB, raw shell, arbitrary code, unrestricted paths, and unbounded loops remain compatibility-only operations and are not exposed through the contract.

`encoding: "dense"` returns `[stable_ref,label,role,flags]` choice tuples in a short envelope. Flags are `1=clickable`, `2=enabled`, `4=checked`, `8=scrollable`, `16=visible`, and `32=redacted`. Use object encoding only when named fields materially help the adapter.

`android.execute` requires a bounded deadline, a maximum mutation count, and optional identity/generation preconditions. Adjacent semantic steps compile into one bounded helper frame. A completed frame returns a committed receipt; a helper-stopped frame returns `E_PARTIAL` with the exact completed prefix and mutation count; a transport failure after possible mutation returns `E_UNKNOWN_COMMIT`. Observe and re-plan after either non-committed outcome instead of blindly replaying it.

Receipts are operation evidence, not user-goal evidence. A caller must declare
and verify a postcondition after a mutation; a successful `android.execute`
does not prove that a web page loaded, media played, or an app reached the
intended state. Report `failed`/`blocked` when the postcondition or exact
cleanup proof is absent.

If an observation cannot fit its requested transcript budget, the contract
returns `E_OUTPUT_LIMIT` with measured `bytes`, `budget`, `mode`, and a compact
next-step hint. It never returns a successful-looking truncated observation.

The machine-readable source is `references/agent-contract.json`. The Rust implementation is [`contract.rs`](https://github.com/austinintelligence/android-use/blob/main/crates/android-use/src/contract.rs).
