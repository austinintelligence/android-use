---
name: android-use
description: Use one enrolled Android device through plain-English read and act commands.
---

# Android Use

`android_read` and `android_act` each take one `command` string (in a shell: `au read …`, `au act '…'`). Every command form: [protocol](references/protocol.md), or send `android_read help`.

1. Act directly on a label you expect; read `screen` (or `page` for Chrome) only when the state is unknown.
2. Quote the label exactly: `tap "Wi-Fi"`, `type "hi" in "Message" then tap "Send"`, `scroll down`, `open app "Settings"`. Add `number N` for a duplicate.
3. A successful action already shows the new screen; do not read it again.
4. A missing label lists near labels, a `fix:` line to send as is, and what is on screen. Use `tap point X Y` on the attached image only when no label works.

`stale`: nothing ran, retry. `partial` or `unknown`: read first, never replay. Ask before deleting, buying, sending, account changes, camera, microphone, location, or recording. No shell or page JavaScript.

[Safety](references/safety.md) · [Setup](references/setup.md)
