---
name: android-use
description: Use one enrolled Android device through bounded plain-English read and act commands.
---

# Android Use

Android Use lets an agent read an Android screen or Chrome page, act by accessible label, and verify the result.

Decision rule: read only when the current screen or page is unknown; act directly by label when the goal is clear; use `page ...` commands for Chrome content; use a screenshot and coordinates only after a semantic miss.

Common commands are sent as the required `command` string to `android.read` or `android.act`. Quoted values are runtime data supplied for the user's task; Android Use has no default text or default target:

```text
status
screen
screen changes
screen matching "TEXT"
find "TEXT"
tap "TARGET"
toggle "TARGET"
type "TEXT" in "FIELD"
scroll down in "SCROLL AREA"
open app "DISPLAY NAME"
page open "https://example.invalid"
page text matching "SEARCH TEXT"
page click "TARGET"
page type "TEXT" in "FIELD"
wait for text "EXPECTED TEXT" up to 5 seconds
verify text "EXPECTED TEXT" exists
capture screen
```

Join a short sequence with `then`, outside quotes: `type "TEXT" in "FIELD" then tap "TARGET" then verify text "EXPECTED TEXT" exists`.

Normal results are short: `Done. Tapped Save.` or a compact screen/page summary. If a label is duplicated, Android Use names the candidates and asks for `tap "Save" number 1` or another number. `stale` means the screen changed before acting; retry after the returned refresh. `partial` means some actions ran; read before another mutation. `unknown` means dispatch may have happened; read and reconcile, never blindly replay. A permission result tells you which Android approval is missing. If a semantic target is absent, a current screenshot may be attached; use a fresh bounded `tap point X Y` only when necessary.

Ask before deletion, purchases, account changes, submissions, notification actions, location, or camera, microphone, and screen recording. Never request shell commands or arbitrary page JavaScript.

Advanced grammar: [protocol](references/protocol.md). Safety: [safety](references/safety.md). Setup: [setup](references/setup.md).
