# Token budget

An agent pays for Android Use three ways: the fixed text it carries on every call (instructions and tool definitions), the results it reads, and the number of calls it makes. Each is designed down and gated. The measured effect is in [measurements](../design/measurements.md).

## Fixed cost: 396 tokens a turn

- **Two tools, one string each.** `android_read` and `android_act` take a single `command`; the schemas are minimal, and the descriptions come from one table (`computer/src/spec.rs`) that also generates `android_read help`, the installed protocol reference, and the grammar tests. They cannot drift apart.
- **Short instructions.** The server's `instructions` and `SKILL.md` (263 tokens) say the operating loop once.

## Result size: say only what is new

- **Dense screen rows.** `sw Wi-Fi=on`, `edit Email="x"`, `Save#2`; plain names for buttons and text; descriptive subtitles only in `screen full`. Labels stay verbatim so the agent can quote them back.
- **`Done.` and `Changed:`.** A fully successful action returns the new screen, or, when it is the screen the agent last saw, only the rows that changed (`~` changed, `+` added, `-` removed).
- **Images only when needed.** A screenshot is attached when a screen has little readable text. It is classified (dense text, ordinary UI, sparse), resized to what a vision model keeps anyway, quantized to an indexed PNG, and held to a 350 KB budget; most land at 20 to 200 KB with a numbered box per control, so the agent taps `e4` instead of guessing coordinates.

## Fewer calls

- **Chains.** `open app "Settings" then tap "Network & internet" then verify text "Airplane mode" exists` is one call. Later steps resolve against the screen the earlier ones produced, even across an app or page load.
- **The helper settles.** After a tap it waits for the screen to react and go quiet before replying, so the result is already the settled screen and the host never polls.
- **Reads inside chains.** `find "TEXT"` or `screen` can end a chain, so an agent can act and look in one call.
- **Handles and near-misses.** Unlabeled controls get stable numbers (`e7`), and a label that is a safe near match is used and reported rather than failing and costing a retry. Risky words never match loosely.
- **`scroll … until "TEXT"`** replaces a loop of scrolls and reads.

## Gates

`cargo xtask verify` measures all of it on every change: fixed tokens must stay at or under 420, the seven recorded screens at or under 360, and each agent-facing document under its own token cap. `cargo xtask benchmark-live` measures calls, tokens, time, and success on a device. When a change makes a number worse, the gate fails and the change either improves or explains the number. See [testing](../contributing/testing.md).
