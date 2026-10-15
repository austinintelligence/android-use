# Security model

[SECURITY.md](../../SECURITY.md) is the policy: supported versions, how to report a vulnerability, and the exact guarantees. This page explains the model behind it.

## Who is trusted with what

| Party | Can do | Cannot do |
| --- | --- | --- |
| **You** | Approve USB debugging, turn on the accessibility service, grant optional permissions, install or remove Android Use. | |
| **The agent** | Send `android_read` and `android_act` commands: look at the screen, tap, type, scroll, open apps, drive Chrome, request captures. | Run a shell or `adb`, evaluate page JavaScript, enable the service, approve an Android prompt, or reach a second device. |
| **`au` (your computer)** | Plan, journal, and send bounded plans to one enrolled device. | Act without a connected, enrolled device; send a plan against a screen that has changed. |
| **The helper (the device)** | Read the accessibility tree and perform the plan's gestures. | Use the internet (no `INTERNET` permission), accept connections from anything but the ADB shell user, or hand its key to any other app, or run a plan it has not fully validated. |
| **Android** | Owns every sensitive prompt and permission. | |

What an agent can do is exactly what you could do by tapping, so give it only the device state, accounts, and apps you would let a helper use. A phone signed in to your bank is a phone an agent can operate; use an [emulator](../guides/emulators.md) when that is not what you want.

## What each layer protects

- **ADB trust** (the "Allow USB debugging?" prompt) decides which computers can talk to the device at all. Revoke it in Developer options when you stop using Android Use.
- **Enrollment** binds a target to one hardware identity (a serial, or for emulators a per-target identifier). Every connection re-checks it, and an ambiguous or missing identity fails closed. A stale record never selects or stops a different device.
- **Helper identity** proves the process on the helper socket is the real helper. Any app can bind an abstract socket name first, and the helper only trusts the shell user, so the helper alone cannot stop a squatter. Instead `au` sends a random challenge on each of the helper's two sockets, and the helper answers with an HMAC under a per-process key that only the ADB shell user can read. `au` checks the answer in constant time before it sends anything else, and refuses a peer that answers wrongly or not at all. Code running as the shell user or root can still read the key.
- **Generation checks** make a plan apply only to the screen it was planned from. If the screen changed, nothing runs.
- **The journal** records each plan before it is sent. After a crash or timeout, new actions are refused as `unknown` until the agent reads the device, so an uncertain action is never repeated by accident.
- **Bounds** limit frames, plans, text, waits, captures, and every response, so a misbehaving app or agent cannot exhaust the host.
- **No escape hatches**: no shell, no page JavaScript, an allowlist of settings screens, and links limited to `http(s)`, `geo:`, and `google.navigation:`.

## What it does not protect against

- A computer that is already compromised: it holds the ADB key and can drive the device without Android Use.
- What the screen says. App and web text is untrusted input that may try to instruct the agent; the protocol tells agents to ask before destructive, account, purchase, submission, and capture actions, but a model can still be fooled. Keep a human in the loop for those.
- A network bridge you add around `au serve`: it speaks stdio and listens on nothing, so authenticating and encrypting any bridge is on you. [Production guidance](../guides/production.md) lists what to add.

## Sensitive data

Screenshots, notifications, and page text can hold credentials and messages. They reach the agent only as command results. Large captures stay in a private artifact folder (mode 0700, deleted after 7 days); the journal keeps operation metadata and digests, not content. Do not put the state folder in a synced or shared directory.
