# Remote broker (E7): design for review

Status: proposal. Nothing in this document is built, and `au serve` stays stdio-only until this is approved.

## Goal

Let an agent running on one machine drive an emulator (or a device) attached to another machine, without weakening the guarantees `au serve` gives today: one enrolled device per session, generation-checked plans, at-most-once actions, no raw ADB.

## Non-goals

A multi-tenant service, user accounts, a hosted cloud of emulators, or any path that lets a remote caller reach ADB, a shell, or Android consent screens.

## Shape

A small `au broker` process runs next to the emulators (the "host"). The agent's machine runs `au serve --mcp --broker URL` (the "client"). The client keeps today's MCP surface and command grammar; only the leg between `au` and the device changes.

```
agent ── stdio MCP ── au serve (client) ── authenticated channel ── au broker (host) ── adb ── emulator
```

The broker exposes the same two operations the local server uses internally: run a read command, run an act command. It never exposes `adb`, `emulator`, or file access. Command text is parsed and resolved on the host, where the journal and fence live, so at-most-once behavior is unchanged.

## Authentication and transport

Decision needed from the owner; the recommendation is the first.

1. **SSH or a tunnel the owner already runs (recommended).** The broker binds `127.0.0.1` only and requires a per-target bearer token. Remote access is the owner's own SSH `-L` forward, WireGuard, or Tailscale. Android Use never terminates TLS or opens a public port, so there is no new internet-facing surface to get wrong.
2. **Built-in TLS with pinned certificates.** The broker listens on a configured address with a self-signed certificate; the client pins its fingerprint. More self-contained, but it makes Android Use responsible for a network listener.

In both cases:

- Tokens are 256-bit random, created by `au broker token NAME`, stored hashed on the host, shown once, and scoped to one target.
- One token, one target, one live session: the existing exclusive serve lock applies, so a second client is refused.
- Tokens expire (default 8 hours) and can be revoked with `au broker revoke`.
- The broker refuses to start on a non-loopback address unless TLS is configured.

## What stays enforced on the host

Everything in [SECURITY.md](../../SECURITY.md) and [agent protocol](../reference/agent-protocol.md): the enrolled-identity check, generation checks, bounded plans, the journal and fence, `unknown` after an interrupted action, and Android's own consent prompts. A remote client cannot enable Accessibility, grant permissions, or approve a prompt.

Private content leaves the host only as the command results the agent already receives today. Screenshots are image content in the reply, as now; audio, recordings, and diagnostics stay on the host and are returned as handles.

## Threats considered

| Threat | Mitigation |
| --- | --- |
| Token theft | Short expiry, per-target scope, hashed at rest, loopback-only by default, revocation. |
| Replay of a captured request | Per-connection random nonce and monotonically increasing sequence, matching the helper's own rule. |
| Port scanning or public exposure | No non-loopback listener without TLS; no default listener at all. |
| A second client hijacking a session | Exclusive serve lock per target. |
| Resource exhaustion | The existing frame, queue, and plan bounds; one session per target; a global session cap. |
| Broker used to run arbitrary host commands | The protocol carries only read/act command strings; there is no exec, file, or ADB operation. |
| Prompt injection from screen content | Unchanged: screen text is data, and the agent protocol still requires approval for destructive, account, purchase, and capture actions. |

## Work if approved

1. `au broker serve [--listen 127.0.0.1:PORT]`, `token`, `revoke`, `status`: about 300 lines of Rust.
2. Client mode for `au serve --broker`: swap the local device handle for a remote one behind the same trait.
3. Tests: auth failure, expiry, revocation, two clients on one target, loopback refusal, replay.
4. Docs, `SECURITY.md` update, and a budget increase disclosed in the changelog.

## Open questions for the owner

- Transport: tunnel-only (option 1) or built-in TLS (option 2)?
- Should the broker serve physical devices, or emulators only?
- Is a global cap of one session per broker enough for the first release?
