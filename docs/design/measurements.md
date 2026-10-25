# Measurements

What Android Use costs an agent, how we measure it, and the numbers behind the claims in the README. The raw runs and scripts are in [`baseline/`](baseline/).

## Setup

- **Device:** an API 36 `google_apis` arm64 emulator (Pixel 7 profile), headless, on Apple silicon. Real-device latency differs; the call and token counts do not.
- **Harness:** `cargo xtask benchmark-live` drives `au serve --mcp` over stdio, the way an agent client does. The "before" numbers come from [`baseline/bench.py`](baseline/bench.py), the same tasks and policy run against the code before the 1.1 changes.
- **Agent policy:** scripted and reproducible. An action quotes only labels already seen, chains across seen labels, stops scrolling once the target shows, and when a step misses because the screen has not caught up it reads and retries at most twice. That makes it a lower bound on what a real model spends.
- **Tokens:** counted with the o200k tokenizer as a stand-in. Ratios matter; absolute counts vary by model. `baseline/trace_tokens.py` recounts a `BENCH_TRACE` file.
- **Tasks:** thirteen over Settings, the `aubench` fixture app, and Chrome, ten runs each; the nine that exist in both harnesses are compared.

## Results

| Metric | Before | Now | Change |
| --- | --- | --- | --- |
| Fixed tokens per turn (instructions + both tool definitions) | 834 | 396 | **−53%** |
| Tool calls per task (shared nine) | 5.9 | 2.6 | **2.3× fewer** |
| Result tokens per task | 426 | 142 | −67% |
| Input tokens per task (results + fixed cost per call) | 5,356 | 1,186 | **−78%** |
| Output tokens per task | 117 | 53 | −55% |
| Success rate | 64% (29/45) | **98% (127/130)** | +34 points |
| `screen` listing, seven recorded screens | 543 | 347 | −36% |
| `SKILL.md` loaded per session | 548 | 263 | −52% |
| Cold `au read status` | 150 ms | 70 ms | −53% |
| Tap p50 / p95 | 686 / 1,654 ms | 695 / 2,164 ms | unchanged |
| Helper APK | 52,454 B | 39,200 B | −25% |

## Not met

- **Screen listings fell 36%, not the 50% we aimed for.** After dropping tags, subtitles, and wrappers, what is left is the app's own text, which must stay verbatim so the agent can quote it back. The saving that matters comes after an action: a `Changed:` diff or `Done.` instead of a full listing (−67% on what each action returns).
- **Action latency did not improve.** The helper's settle (wait for the screen to react, then 200 ms of quiet) replaced the host's polling and has the same floor. The saving is in round trips and process spawns, not wall time per tap.
- **Three tasks missed once in ten** (`find_setting_scroll`, `toggle_wifi`, `settings_search`): Settings screens that finish loading after the screen goes quiet. The next read recovers them, as the harness's agent does. On a cold, just-booted emulator a tap into a sub-page can return the previous screen for the same reason.

## The screen format

A plain name is a button or text; a prefix marks other controls (`sw`, `edit`, `chk`, `radio`, `tab`, `link`, `list`); `=` carries a control's value, `:` an item's short state; `#N` marks a duplicate. Descriptive subtitles move to `screen full`. Seven real screens went from 543 to 347 tokens.

Settings home, before (88 tokens):

```text
Search Settings
Google — Services & preferences
Network & internet — Mobile, Wi‑Fi, hotspot
Connected devices — Bluetooth, pairing
Apps — Assistant, recent apps, default apps
Notifications — Notification history, conversations
Sound & vibration — Volume and haptics
Modes — Do Not Disturb, Bedtime
```

After (29 tokens):

```text
Search Settings
Google: Services & preferences
Network & internet
Connected devices
Apps
Notifications
Sound & vibration: Volume and haptics
Modes
```

## Gates that keep it this way

`cargo xtask verify` fails the build if: fixed tokens exceed 420 or the recorded screens exceed 360 (`tools/src/tokens.rs`); a documentation file exceeds its token budget (the agent-facing ones are capped between 130 and 1,800 tokens each); the helper APK exceeds 60,000 bytes or the host binary 1,120,000 on macOS, 1,200,000 on Windows, or 1,450,000 on Linux (CI also checks the binary on all three with `cargo xtask binary-size`); the Rust, Java, automation, or total source line budgets are exceeded; or product code mentions `uiautomator`. `cargo xtask benchmark-live` with `BENCH_BASELINE` and `BENCH_GATE=1` also fails on a lower success rate or a p95 more than 20% slower.

## Reproduce

```sh
cargo xtask android            # debug helper and the aubench fixture
BENCH_RUNS=10 BENCH_TRACE=target/trace.json cargo xtask benchmark-live
python3 docs/design/baseline/trace_tokens.py target/trace.json
```

The device or emulator needs the helper enabled and the fixture installed; [testing](../contributing/testing.md) explains both.
