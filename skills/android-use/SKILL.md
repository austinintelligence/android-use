---
name: android-use
description: Fast bounded semantic Android control through Android Use v3.
---

# Android Use

Use `android.read` to inspect the bound Android device.
Use `android.act` for bounded mutations.
Use `android.read` with `q=browser` and `op=tabs|observe|text` for Chrome state.
Use `android.read` with `q=capabilities|location|notifications` for compact device state, and `q=visual` with `op=hash|diff` for host PNG artifacts. Location is a bounded one-shot request with a graceful unavailable response.
Use `android.act` with `target=browser` for generation-guarded CDP plans.
Observe before acting.
Pass the returned `g` generation to `android.act`.
Prefer returned integer refs over text matching.
Keep plans short, linear, and deterministic.
Use `wait` or `assert` for the immediate expected outcome.
After success, observe only when the task requires confirmation.
On `stale`, observe again and rebuild the plan.
On `partial`, do not repeat the plan; observe first.
On `unknown`, never repeat the operation ID blindly; observe first.
For browser work, use CDP page operations after tab discovery; keep Android accessibility for Chrome's own navigation chrome.
Use `camera` or `microphone` only in an explicit plan after permission/capability checks. Use `screen_record` only when the device reports that screen-record permission is available. Notification plans support open, dismiss, and a single safely identifiable primary action. A visual plan with `target=visual` supports bounded PNG crop.
Artifacts are private handles; fetch only the required bounded range.
Normal UI output is a bounded semantic frontier.
Request detail only when the frontier is insufficient.
The selected device is fixed for the server session.
Do not request raw ADB, shell, installs, downloads, loops, or branches.
Ask before deletion, account changes, purchases, submissions, or privacy-sensitive capture.

See [protocol](references/protocol.md), [safety](references/safety.md), and [setup](references/setup.md).
