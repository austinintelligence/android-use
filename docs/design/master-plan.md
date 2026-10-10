# Android Use master plan: cheapest, fastest, easiest Android tool for agents

Status: **approved and implemented through Step 6 (see §8). Steps 7–8 are described below.**

This extends the earlier master plan ([`android-use-final-plan.md`](android-use-final-plan.md)). It does not replace it:

- Its items A1–A20, Q1–Q4, S, P2–P4 and E1–E7 keep their names and their definitions.
- This file adds Goals 1–6, reorders the steps, and records what changed after the plan was first written.
- The earlier package's decision register (D1–D18), test matrix and emulator design (C.1–C.9) still apply unless a row below amends them.

Baseline data, the measurement harness and the recorded screens are in [`baseline/`](baseline/).

---

## 0. Step 0: done (2026-10-09)

The work was committed in slices and pushed to `origin/improve/agent-native-reliability`. No PR has been opened yet; opening one is the owner's call.

| Commit | What |
| --- | --- |
| `62e8647` … `4577c8b` | The session's work in five slices: journal/engine, agent layer, compression, helper, docs and CI |
| `43fbd7f` | **New P0, found live.** The helper crash-looped on every Android 14+ device. `startForeground()` without a type claims every manifest type, and claiming `mediaProjection` before consent throws. 1.0.0 shipped this bug |
| `ce5413c` `59a015d` `d18c594` `1cd32fe` + merges | The 27 confirmed regression-review findings, fixed in four isolated worktrees. Two of them were critical: later plan steps acting on a covered screen (#14), and a point tap mapped through an image the next step had made stale (#20) |
| `c4e0a76` | **Live fix to #14.** Comparing exact bounds stopped `type … then tap "Submit"` as partial, because the keyboard resized the layout. A dialog that opens over the target still stops the plan; this was checked on the device |
| `d73225f` | The size gate now counts `*_tests.rs` as tests. Rust production is 6,643, under the original **6,700** limit, so the raise to 6,900 was reverted. The authored-code limit is now 13,300 (it includes tests) |

Gate: 98 Rust tests, 18 JVM tests, clippy, `gjf --aosp`, the docs gate and `cargo xtask verify` all pass.

---

## 1. Baseline (deliverable 1)

**Setup.** API 36 `google_apis` arm64 emulator (Pixel 7 profile, 1080×2400), headless, Apple Silicon. Branch `improve/agent-native-reliability` at `c4e0a76`; host `dist` build and a debug helper.

**Harness.** [`baseline/bench.py`](baseline/bench.py) drives `au serve --mcp` over stdio, exactly as a client does.

- **Token counts** use the o200k tokenizer as a proxy for the model's tokenizer. Ratios are what matter; absolute numbers will differ by tokenizer.
- **Agent policy ("informed + recover"):** an action only quotes labels already seen, chains only across seen labels, and stops scrolling once the target shows. When an action misses because the screen has not caught up, it reads and retries at most twice. Scripted and reproducible, this is a *lower bound* on what a real model spends.
- **Tasks.** Nine tasks over Settings and the `aubench` fixture, five runs each.
  - Chrome tasks are **blocked**: Chrome's first-run terms on the emulator must be accepted first, and that needs the owner's OK (see §7).
  - The duplicate-label task is **blocked**: the fixture has no duplicate labels, so Q2 adds them.

### 1.1 Fixed per-turn cost (paid on every model call)

| Part | Bytes | Tokens |
| --- | --- | --- |
| `INSTRUCTIONS` (sent at initialize) | 704 | 143 |
| `android.read`: description 128 + schema 35 + JSON wrapper | 587 (description) | 210 |
| `android.act`: description 379 + schema 35 + JSON wrapper | 1,521 (description) | 481 |
| **Total fixed** | | **834** |
| Skill bundle (`SKILL.md` 548 + references 478) | 4,325 | 1,026 |

### 1.2 Tasks (recover policy, merged branch, 5 runs each)

| Task | Success | Calls | Input tok | Output tok | Result tok | Wall ms |
| --- | --- | --- | --- | --- | --- | --- |
| toggle Wi‑Fi | **0/5** (S1) | 10.6 | 9,904 | 197 | 1,064 | 8,269 |
| find setting with scrolling | 1/5 | 6.4 | 5,638 | 114 | 300 | 4,777 |
| fill and submit form | 3/5 | 4.4 | 3,999 | 109 | 330 | 4,904 |
| dismiss dialog | 5/5 | 5.0 | 4,545 | 106 | 375 | 6,340 |
| confirm dialog | 5/5 | 4.6 | 4,202 | 98 | 366 | 7,013 |
| toggle switch | 5/5 | 2.8 | 2,592 | 56 | 257 | 5,388 |
| scroll to row 40 | **0/5** (R3) | 5.2 | 4,671 | 113 | 334 | 6,968 |
| Settings search | 5/5 | 12.2 | 10,953 | 228 | 778 | 8,586 |
| status + screen | 5/5 | 2.0 | 1,696 | 32 | 28 | 8 |
| **Mean** | **29/45 (64%)** | **5.9** | **5,356** | **117** | **426** | **5,806** |

The strict policy (no recovery, before the merge, 3 runs) gave 37% success and 3.6 calls. Both runs are in `baseline/`.

**Where the input tokens go:**

- About **91%** of input is the fixed 834 tokens, repeated on every call. Only about 9% is what the device returned.
- So cutting **calls** and **fixed text** dominates; screen size comes third.

**Per-action latency, p50 / p95 (ms):**

| Action | p50 | p95 |
| --- | --- | --- |
| open | 733 | 1,937 |
| tap | 686 | 1,654 |
| type | 324 | 799 |
| scroll | 45 | 1,103 |
| screen read | 10 | 38 |
| status | 1 | 2 |

Actions include the host-side settle.

### 1.3 New defects found live (not in the earlier plan)

| Id | Defect | Effect on the benchmark | Priority |
| --- | --- | --- | --- |
| **R1** | After `open app`, `Now on screen:` is often still the **launcher**: settle returns before the app's window appears | The agent's next label misses, and each miss costs a read plus a retry. This is the single largest source of extra calls | P0 |
| **R2** | Reads during a transition return "The screen has no readable controls" | The same extra calls | P0 |
| **R3** | List rows are merged into a neighbouring text's value (`Ready — Fixture row 1, Fixture row 2, …`). The agent sees "Fixture row 40", but `tap "Fixture row 40"` fails | scroll to row 40 fails 0/5; wasted tokens | P0 |
| **S1** | The Settings **Wi‑Fi switch row is missing** from `screen`, `screen full` and `find`. `uiautomator` sees it at `RecyclerView > LinearLayout* > RelativeLayout > TextView "Wi-Fi"` + `Switch` | The README's headline `tap "Wi-Fi"` cannot work on Android 16 | P0 |
| R4 | `scroll down` on a screen that cannot scroll says "Done. Scrolled down." | Wasted calls in scroll loops | P1 |
| R5 | `au doctor` reports "ready" while the helper crash-loops (it never sends a bridge hello) | The FGS crash went unnoticed | P1 |
| R6 | `status` reports Accessibility off for about 2 s after `au update` (rebind race) | Spurious failure | P2 |
| R7 | `screen full` returned exactly the same text as `screen` on all four recorded screens | `full` adds nothing | P2 |
| R8 | A text field whose hint equals its label prints the label twice (`Fixture text — Fixture text text field`) | Tokens | P2 |

---

## 2. Screen format before/after (deliverable 2)

These are real recordings from the emulator (`baseline/screens/*.txt`). Counts use o200k. Reproduce with `python3 baseline/format_proto.py baseline/screens`.

**Proposed format (V5):**

- A kind prefix only for controls: `btn`, `sw`, `edit`, `chk`, `radio`, `tab`, `link`, `list`, `slider`. Plain text has none.
- Values after `=`, or `:` for item values.
- `#N` only on duplicate labels.
- **Labels stay verbatim**, so the agent can quote them back exactly.
- Short, state-like values are kept (`Off`, `None`, `T-Mobile`, `AndroidWifi`).
- Descriptive subtitles (more than two words, or a comma list) are dropped from `screen` and kept in `screen full`, which also fixes R7.

| Screen | Current | V1 prefix only | V2 + subtitles cut | V3 V2 + `eN` on every row | **V5 (proposed)** |
| --- | --- | --- | --- | --- | --- |
| Settings home | 88 | 88 | 85 | 105 | **29 (−67%)** |
| Settings › Network & internet | 58 | 57 | 55 | 75 | **49 (−16%)** |
| aubench fixture | 86 | 78 | 44 | 58 | **35 (−59%)** (+R3/R8 fixes) |
| aubench dialog | 18 | 15 | 15 | 23 | **15 (−17%)** |
| **Total** | **250** | 238 (−5%) | 199 (−21%) | 261 (+4%) | **128 (−49%)** |

**Settings home.** Current (88 tokens):

```text
Search Settings
Google — Services & preferences
Network & internet — Mobile, Wi‑Fi, hotspot
Connected devices — Bluetooth, pairing
Apps — Assistant, recent apps, default apps
Notifications — Notification history, conversations
Sound & vibration — Volume and haptics
Modes — Do Not Disturb, Bedtime
Display & touch — Dark theme, font size, touch
Wallpaper & style — Colors, themed icons, app grid
```

V5 (29 tokens; the subtitles stay available through `screen full`):

```text
Search Settings
Google
Network & internet
Connected devices
Apps
Notifications
Sound & vibration
Modes
Display & touch
Wallpaper & style
```

**Network & internet.** Current (58 tokens) vs V5 (49 tokens): state is kept, and the long subtitle is dropped.

```text
Network & internet                           Network & internet
Navigate up — button                         btn Navigate up
Internet — AndroidWifi                       Internet: AndroidWifi
SIMs — T-Mobile                              SIMs: T-Mobile
Hotspot & tethering — Off                    Hotspot & tethering: Off
Data Saver — Off                             Data Saver: Off
VPN — None                                   VPN: None
Private DNS — Automatic                      Private DNS: Automatic
Adaptive connectivity — On                   Adaptive connectivity: On
Mobile network security — Network type, …    Mobile network security
```

**aubench fixture.** Current (86 tokens) vs V5 (35 tokens). After R3, each row is its own line and tappable, so the list is shown as `list Benchmark scroll surface (rows 1–8 of 60, more ↓)` instead of one merged value.

```text
AU agentic benchmark fixture                  AU agentic benchmark fixture
Benchmark scroll surface — scroll area        list Benchmark scroll surface
Fixture text — Fixture text text field        edit Fixture text=""
SUBMIT FORM — button                          btn SUBMIT FORM
Benchmark toggle — off switch                 sw Benchmark toggle=off
OPEN BENCHMARK DIALOG — button                btn OPEN BENCHMARK DIALOG
Ready — Fixture row 1, Fixture row 2, … (×8)  Ready
```

**Diffs after acting** (same screen identity, only what changed):

| Action | Today (`Now on screen:`) | Diff | Saving |
| --- | --- | --- | --- |
| `tap "Benchmark toggle"` | 86 | `~ sw Benchmark toggle=on` = **6** | −93% |
| `type … then tap "SUBMIT FORM"` | 87 | 56 today; about 15 after R3 (`~ edit Fixture text="hello"` / `~ Submitted: hello`) | about −83% |

When the screen identity changes (another window title, package or activity), the full V5 screen is returned instead.

**`eN` handles: measured, and the design changed.** Putting `eN` on every row costs **+23–68%** screen tokens (V3). So:

- Handles are **accepted everywhere** (`tap e12`).
- Handles are **printed only where a label cannot do the job**: unlabeled controls, which today are invisible, and the set-of-marks overlay on images.
- Duplicates keep `#N`, which is shorter than a handle.

---

## 3. `au install <agent>` design (deliverable 3)

### 3.1 Principles

- **Never rewrite a user's file wholesale.**
  1. Parse the file.
  2. Change one entry, `android-use`.
  3. Write it to a temp file in the same directory, then rename it over the original atomically.
  4. Keep the previous file as `<file>.au-backup` (one generation).
- **Idempotent.** An identical entry is a no-op that reports "already installed".
  - Our own entry (command basename `au`/`au.exe`, args `serve --mcp`) is updated in place.
  - A **different** `android-use` entry we did not write is left alone, with an error naming `--force`.
- **`--dry-run`** prints the exact diff or the command it would run and writes nothing. `--json` gives a machine-readable report: client, file, action, backup path.
- **Prefer each client's own CLI** when it is on PATH (`claude mcp add`, `codex mcp add`, `gemini mcp add`, `code --add-mcp`). Fall back to a file edit only when that CLI is missing.
- **Absolute path for GUI clients.** Desktop apps launched from the Dock or Start menu do not inherit the shell's PATH: on macOS, Homebrew's `/opt/homebrew/bin` is invisible to Claude Desktop, Cursor and VS Code.
  - So `au install` writes the absolute path of a **stable shim**: the Homebrew symlink, the npm global bin, or `~/.local/bin/au` from the script installer. It never writes a versioned path.
  - Terminal clients (`claude`, `codex`, `gemini`, `opencode`) get bare `au` when `which au` resolves to that same shim.
- **JSON with comments** (VS Code `mcp.json`, `opencode.jsonc`) cannot round-trip through serde. If the file has comments or trailing commas, `au install` does not edit it; it prints the snippet to paste and exits with `next_step`.
- **Key order.** Strict JSON is edited with `serde_json`'s `preserve_order` feature, an existing crate plus `indexmap`, so key order is kept.
  - This is the one dependency change, and it has a measured reason: without it serde reorders every key in the user's file.
  - If the owner declines it, the fallback is a surgical text splice of one entry, at about +60 lines.
- **TOML** (Codex): no TOML crate. Replace or append exactly our `[mcp_servers.android-use]` table, the lines from its header to the next table header, and never touch other tables. If the file does not pass a minimal table scan, print the snippet instead.

### 3.2 Per client

| Client | MCP registration | Skill | Uninstall |
| --- | --- | --- | --- |
| `claude` (Claude Code) | `claude mcp add android-use --scope user -- au serve --mcp`; fallback `~/.claude.json` → `mcpServers.android-use` | `~/.claude/skills/android-use/` (SKILL.md + references) | `claude mcp remove android-use --scope user`; delete the skill dir only if its `.au-installed` marker matches |
| `codex` | `codex mcp add android-use -- au serve --mcp`; fallback `~/.codex/config.toml` → `[mcp_servers.android-use] command="au" args=["serve","--mcp"]` | `~/.codex/skills/android-use/` **[validate]** that this Codex version loads user skills | `codex mcp remove android-use`, or table removal; skill dir by marker |
| `cursor` | `~/.cursor/mcp.json` → `mcpServers.android-use {command, args}` (absolute path) | none: Cursor has no skill folder. The MCP `instructions` carry the guidance | Remove the entry |
| `gemini` (Gemini CLI) | `gemini mcp add android-use au serve --mcp` **[validate flags]**; fallback `~/.gemini/settings.json` → `mcpServers.android-use` | Optional `~/.gemini/extensions/android-use/` with `gemini-extension.json` + `GEMINI.md` = SKILL.md **[validate]** | Remove the entry and extension dir |
| `vscode` | `code --add-mcp '{"name":"android-use","command":"<abs au>","args":["serve","--mcp"]}'`; fallback user `mcp.json` (macOS `~/Library/Application Support/Code/User/`, Linux `~/.config/Code/User/`, Windows `%APPDATA%\Code\User\`) → `servers.android-use {type:"stdio", …}` | none | Remove the entry |
| `claude-desktop` | `claude_desktop_config.json` (macOS `~/Library/Application Support/Claude/`, Windows `%APPDATA%\Claude\`) → `mcpServers.android-use` (absolute path) | none (Desktop has no user skills folder) | Remove the entry |
| `opencode` | `~/.config/opencode/opencode.json` → `mcp.android-use {type:"local", command:["au","serve","--mcp"], enabled:true}` | `~/.config/opencode/skills/android-use/` **[validate]** | Remove the entry |
| `all` | Every client detected below | | |
| (none given) | Auto-detect: a client counts as present if its CLI is on PATH **or** its config directory exists. Show the list, then install to each one; `--yes` skips the confirmation prompt | | |

Every config entry uses the server name `android-use` and the arguments `["serve","--mcp"]`. Tool names change to `android_read` / `android_act`. The dotted names break the function-name rules of some model APIs (see [`android-use-review.md`](android-use-review.md), recommended next steps). The old dotted names are still **accepted** in `tools/call` but are not listed, so the fixed token cost does not grow.

**Files:**

- New `computer/src/clients.rs`: about 260 lines, with a table-driven client list.
- `main.rs`: `install` / `uninstall <agent>` routing. Bare `uninstall` keeps today's behaviour.
- Tests: `computer/tests/clients.rs`, using temp `HOME`/`APPDATA` fixtures.

**Pass criteria:**

- Round-trips over fixture configs with other servers present: the other entries come back byte-identical.
- Running install twice is a no-op.
- `--dry-run` writes nothing.
- Uninstall removes only our entry.
- Comment-bearing JSONC prints a snippet instead of editing.

### 3.3 Embedding the helper APK

- **Build.**
  1. The release workflow builds and signs `aubridge.apk` first.
  2. The host build sets `AU_EMBED_APK=<path>`.
  3. `build.rs` reads the APK, computes its SHA-256 and emits `include_bytes!` plus a `const APK_SHA256`.
  4. Without `AU_EMBED_APK` (dev builds), today's "APK beside the binary" lookup still applies.
- **At install time.**
  1. Re-hash the embedded bytes and compare them with `APK_SHA256`, catching a corrupt binary.
  2. Write the APK to a private temp file.
  3. Check the signer against `device/release-signer.sha256`, which the release pipeline already pins.
  4. Then `adb install`.
- **Measured size cost.** The release R8 APK is **49.7 KB**; the `dist` `au` is **871 KB**, so embedding adds about **+5.7%**.
  - This conflicts with the emulator plan's gate K.7 ("+3% of the 1.0.0 dist build").
  - Proposal: amend K.7 to "+3%, excluding the embedded helper".
  - APK shrinking in Goal 6 should take back part of it.

### 3.4 Distribution channels

**Every publish step needs the owner's approval** (§7).

1. **npm.**
   - `android-use` becomes a thin launcher, already in `install/`.
   - Optional per-platform packages hold only the binary, which now carries the APK:
     - `@android-use/darwin-arm64`
     - `@android-use/linux-x64`
     - `@android-use/win32-x64`
     - and later `linux-arm64` and `darwin-x64`, which today have no release build.
   - The launcher currently allows only `setup/repair/doctor/status/update/uninstall`. It must also pass through `serve`, `read`, `act`, `install` and `version`, so that `npx -y android-use serve --mcp` works as a zero-install MCP command.
   - `npx -y android-use setup` runs setup **and then** `au install` (Goal 5.5).
2. **Homebrew.** A tap, `austinintelligence/tap`, with formula `android-use` that installs the release archive and verifies `SHA256SUMS`. It depends on `android-platform-tools` as a cask suggestion, not a hard dependency.
3. **Script installers.**
   - `curl -fsSL …/install.sh | sh` and `irm …/install.ps1 | iex`.
   - Each downloads the archive and `SHA256SUMS` from the release, verifies the archive with `sha256sum` / `Get-FileHash` before extracting, installs to `~/.local/bin` or `%LOCALAPPDATA%\AndroidUse\bin`, and offers to add that directory to PATH.
   - The scripts are short and auditable, and are pinned per release tag.
4. **Plugins.**
   - **Claude Code plugin:** `.claude-plugin/plugin.json` + `.mcp.json` (`npx -y android-use@1 serve --mcp`, so it needs no prior install) + `skills/android-use/`. The marketplace is `.claude-plugin/marketplace.json` **in this repo**, which avoids a new repo; install with `/plugin marketplace add austinintelligence/android-use`.
   - **Codex plugin:** the same shape, if this Codex version supports plugins **[validate]**; otherwise `codex mcp add` plus a skill copy.
   - `npx skills add austinintelligence/android-use`: the repo already has `skills/android-use/` at the conventional path **[validate with the CLI]**.
5. **README.** Replace the paste-in agent prompt with:

   > Run `npx -y android-use setup`, follow its prompts, then use `android_read` / `android_act`.

   The recovery runbook stays in `docs/agents/install.md`.

---

## 4. Goals mapped to work items

New items are numbered G1.x … G6.x. Existing ids are kept where they apply.

### Goal 1: Fewer tool calls per task (target: 5.9 → ≤ 2.9 per task)

| Item | Change | Files | Pass criteria |
| --- | --- | --- | --- |
| G1.1 (R1, R2) | Settle waits until **the expected window is up**:<br>• `open app` waits for the target package in the active window;<br>• taps wait for a window-state or content event, then quiet;<br>• an empty or transitional tree keeps waiting, within the existing limit | `agent.rs` settle, `Vm.java` (expose foreground package + window id in `status`) | Benchmark: zero "launcher shown after open app" in 45 runs; success ≥ 95% on the tasks that are not blocked |
| G1.2 (P2) | **One round trip per intent.** The helper resolves, acts, waits for the gesture completion callback (`dispatchGesture(…, callback)`), settles (120 ms quiet, 1.5 s cap) and returns a row diff plus the screen identity in the receipt. The host keeps `settle()` only when the helper is older | `Vm.java`, `Ui.java`, `SemanticCompiler.java`, `bridge.rs`, `agent.rs` | 1 device round trip per action (counted by a fake helper); tap p50 −30% |
| G1.3 | `scroll until "X" [in "AREA"] [up to N]`: the helper scrolls, checks for the label, and stops at the end of the list; the result says found/not found plus the screen | `command.rs`, `Vm.java`, grammar tests | `find_setting_scroll` and `scroll_to_row_40` in **1 call** after open |
| G1.4 | **Reads inside chains.** Allow `find "X"`, `screen`, `verify`, `wait for` anywhere in a `then` chain. Reads never mutate, so the journal fence is unaffected. The result shows the last read's output | `command.rs`, `agent.rs` | `tap "A" then find "B"` = 1 call; at-most-once tests unchanged |
| G1.5 | **Handles.** `tap e12` / `type "x" in e7` resolve through the helper's stable key (window + view id + class + ancestor path + label) bound to a generation. A changed key is `stale` with fresh candidates. Handles are printed only for unlabeled controls and in set-of-marks images (§2) | `Ui.java`, `resolve.rs`, `format.rs` | Unlabeled icon buttons become targetable; a handle from a stale screen is refused |
| G1.6 (A4) | **Near-miss labels**, as the owner proposed. With one candidate and high overlap (needle ≥ 60% of the label's characters, word-aligned prefix or suffix), act and say `tapped "Save draft" (closest to "Save")`. Otherwise fail with up to three suggestions in the same result. **Safety:** never auto-resolve when the extra words include a risky verb (delete, remove, erase, reset, pay, buy, send, submit, confirm, all) | `resolve.rs`, `format.rs` | Unit table; benchmark task "duplicate/near-miss label" |
| G1.7 (P4, A5) | **Multi-window reading:** `getWindows()` ordered by layer. A dialog or IME dominates and marks background rows as blocked. Also fixes **S1**, the missing Settings switch row, if the cause is window-related; **diagnose S1 first** | `Ui.java`, `SemanticCompiler.java` | S1 fixed: `tap "Wi-Fi"` works on API 36 Settings; dialog tasks need no extra read |
| G1.8 | **`structuredContent`** on `android_read` / `android_act`: `{status, resolved:{label,handle}, generation, changed, screen_id}`. The text stays the primary channel; structured fields are not repeated in the text | `adapter.rs`, `agent.rs` | MCP conformance test; text tokens unchanged or lower |
| G1.9 (R3) | List rows stay **separate, tappable rows**. A collection is summarized as `list NAME (rows a–b of N, more ↓)` | `SemanticCompiler.java` | `tap "Fixture row 40"` after scrolling works |
| G1.10 (R4) | Scroll reports `no change (end of list)` and counts as a non-mutation | `Vm.java`, `format.rs` | Unit test plus benchmark |

### Goal 2: Fewer input tokens (fixed −40%, screens −50%)

| Item | Change | Pass criteria |
| --- | --- | --- |
| G2.1 | **Token gates in `xtask`**, measured like the docs budgets (bytes/4, the same heuristic as `docs_budget`; the o200k numbers live in the benchmark report):<br>(a) `instructions + both tool JSONs`, from the real `initialize` + `tools/list` of a built `au`;<br>(b) `screen` and `Now on screen:` rendered from **golden semantic-row fixtures** (`computer/tests/fixtures/screens/*.json`, captured from the emulator; raw rows exposed by a debug-only `au read 'screen rows'`, or captured in the harness);<br>(c) the installed skill bundle | The gates fail CI on growth. Initial limits: (a) 500 tokens, (b) −50% of the recorded total, (c) 800 tokens |
| G2.2 | **V5 screen format** (§2), with `screen full` keeping the subtitles (fixes R7, R8) | (b) on 4 goldens: 250 → ≤ 125 |
| G2.3 | **Diffs.** On the same screen identity, return only `~`/`+`/`-` rows. A new identity returns the full V5 screen. The diff is capped, and above about 60% changed rows the full screen is sent | Toggle result ≤ 10 tokens; benchmark result tokens −50% |
| G2.4 | **UIFormer-style pruning** in `SemanticCompiler`: merge single-child wrappers; drop text that repeats an ancestor's or sibling control's label; drop decorative nodes (no text, no action) and nodes outside the screen. `screen full` disables pruning | Per-screen tokens on the goldens; no task regression |
| G2.5 | **Fixed text.**<br>• `INSTRUCTIONS` keeps only: the two tools, "act on labels you saw", "results already show the new screen", the stale/partial/unknown rule, and the ask-first list.<br>• Each tool description lists the eight most-used command forms; the rest go to `references/protocol.md`, read on demand.<br>• Schema: one `command` string, no extra description text | (a): 834 → ≤ 500 (−40%), with **no drop in benchmark success** |
| G2.6 (A7) | Hard caps with the **omitted count first** (`12 more rows below; scroll or find "X"`) | Unit tests: the count survives byte truncation |
| G2.7 | **Set-of-marks.** Opt-in images get numbered `eN` boxes drawn on the compressed `ImageSpace` image, using the same keys as G1.5, so the visual fallback produces `tap e12`. The image stays opt-in | Overlay unit test on a fixture PNG; `tap point` usage in benchmark traces → 0 |

### Goal 3: Fewer output tokens

| Item | Change | Pass criteria |
| --- | --- | --- |
| G3.1 | Shorthand only where unambiguous in `command.rs`: `tap e12`, chains, `scroll until`. No unquoted labels: quotes keep labels from colliding with grammar words. Every new form is added to `computer/tests/command_grammar.rs` **and** to a structured fuzzer over `parse_*` (A10) | Grammar corpus generated from `spec.rs` (G6.2); fuzz runs in CI |
| G3.2 | Result wording asks for less: when a result shows the next state, it ends with nothing to do (no "read the screen" advice). Recovery text appears only on errors | Benchmark: zero reads that immediately follow a successful act |
| G3.3 (optional) | A terse-reply snippet for long sessions | Ship it only if the benchmark shows a net saving after its own input cost |

Baseline output is 117 tokens per task, of which about 13 per call is the tool-call wrapper. Most of the saving comes from fewer calls (Goal 1).

### Goal 4: Speed

| Item | Change | Pass criteria |
| --- | --- | --- |
| G4.1 | `benchmark-live` records p50/p95 per action and per task, and fails when an action's p95 regresses more than 20% against the stored baseline on the same machine and profile (the emulator plan's F.4) | Gate in `xtask` |
| G4.2 (P3) | adb smart-socket transport (TCP 5037, no `adb forward` and no subprocess on the hot path); `blob` frames for images and media, which replace chunked fetches; daemon reuse (`au serve` owns the link and the CLI connects to it); a keepalive so idle never turns into `unknown` (A20) | Cold CLI call: 0 adb subprocesses; screenshot = 1 round trip |
| G4.3 | **Never `uiautomator dump`.** Add a gate that greps the product sources for it. This plan used `uiautomator` once, to diagnose S1, from a shell outside the product | Gate |
| G4.4 | Settle tuning after G1.2, driven by measurement (`SETTLE_QUIET`, the cap) | Tap p50 ≤ 400 ms on the reference emulator |

### Goal 5: One-command install

§3 above, as items G5.1–G5.6:

| Item | Scope |
| --- | --- |
| G5.1 | Channels: npm, Homebrew, script installers |
| G5.2 | Embedded APK |
| G5.3 | `au install` / `au uninstall <agent>` |
| G5.4 | Plugins and marketplace |
| G5.5 | `au setup` calls `install` for the clients it detects, then runs a live `status` + `screen` check |
| G5.6 | README cut to two lines |

### Goal 6: Smaller code, the right way

| Item | Change | Pass criteria |
| --- | --- | --- |
| G6.1 (S) | Delete legacy JSON (about −650 lines) **after Q2**. Remove dead code. Note: the two compacted spots from the fix round (`browser.rs` arms, the `Vm.java` table) get re-expanded if review finds them harder to read | Rust production ≤ 6,000, then lower the budget |
| G6.2 | `spec.rs`: one table → the MCP tool descriptions, `references/protocol.md`, and the grammar corpus. `xtask docs` fails on drift | Generated files byte-identical in CI |
| G6.3 | Release artifact: `profile.dist` as is (already `opt-level="z"`, fat LTO, strip). APK: R8 on (it already is). Add `shrinkResources`, drop unused resources, check the APK size | APK and binary size reported in `xtask size`, and gated |
| G6.4 | No new dependencies without a measured reason. The only proposed one is `serde_json/preserve_order` (§3.1) | Owner approval |

---

## 5. Updated ordering

Order of work, with the master plan's steps kept:

1. **Step 1: P0 correctness.** Nothing new ships before these:
   - S1 (Settings switch row);
   - R1/R2 (G1.1, settle on the expected window);
   - R3 (G1.9, separate list rows);
   - **A15** (fail-closed duplicate serials, as the owner required);
   - A16 (boot wait), A8 (microphone), A11 (link-scheme docs);
   - R4, R5 (doctor sends a bridge hello), R6.

   *Pass:* benchmark success ≥ 95% on the tasks that are not blocked; Wi‑Fi task 5/5.
2. **Step 2: measurement infrastructure.**
   - Q2: port the harness into `xtask benchmark-live` (Rust, MCP stdio, the same nine tasks plus Chrome and duplicate-label tasks).
   - Extend `aubench`: duplicate labels, a near-miss pair, an unlabeled icon button, a slow-loading screen, a horizontal carousel, separate list rows.
   - The harness serves a local page for Chrome.
   - G2.1 token gates; G4.1 latency gates; Q1 fake adb + fake helper.

   *Pass:* `xtask benchmark-live --emulator` reproduces §1 within noise; the gates run in CI from golden fixtures.
3. **Step 3: cheap token wins** (one PR each, each measured):
   - G2.5 fixed text;
   - G2.2 V5 format;
   - G2.3 diffs;
   - G2.6 caps;
   - tool rename to `android_read`/`android_act` with aliases.

   *Pass:* fixed ≤ 500; screens ≤ 125 on the goldens; success not lower.
4. **Step 4: fewer calls.**
   - G1.3 `scroll until`;
   - G1.4 reads in chains;
   - G1.6 near-miss;
   - G1.8 `structuredContent`;
   - then **G1.2 (P2)** one round trip;
   - then G1.7 (P4) multi-window, plus G1.5 handles and G2.7 set-of-marks;
   - G2.4 pruning;
   - G3.1 and G3.2.

   *Pass:* calls/task ≤ 2.9 (≥ 2× fewer); input tokens/task −50%.
5. **Step 5: one-command install.** This can run in parallel with Steps 3–4; it touches separate files.
   1. G5.2 embedded APK;
   2. G5.3 `au install`;
   3. G5.5 combined `setup`;
   4. launcher passthrough;
   5. G5.6 README.

   Then, **with owner approval**, G5.1 publishing and G5.4 marketplace.

   *Pass:* on a clean macOS user, `npx -y android-use setup` → device ready → Claude Code and Codex both list `android_read` and `android_act` without any hand-editing.
6. **Step 6: speed.** G4.2 (P3) transport, daemon and blob frames; G4.4 settle tuning.

   *Pass:* a cold CLI call spawns 0 adb processes; per-action p95 at or below Step 4's.
7. **Step 7: smaller.**
   - G6.1 legacy deletion, which Q2 has unblocked;
   - G6.2 `spec.rs`;
   - G6.3 artifact shrink;
   - then lower the Rust budget.
8. **Then the master plan's E track** (targets, identity, emulator lifecycle) as written. E1 is now partly answered by this session:
   - The API 36 `google_apis` arm64 image boots headless with a hand-written AVD, with no `avdmanager` needed (U6, partial).
   - Chrome is preinstalled (U3: yes on API 36).
   - The helper needs the specialUse FGS fix (J3, done).

   The owner's constraint still holds: **one enrolled device per session.**

**Measurement gate for every phase.** Run `benchmark-live` on `aubench` plus the local page. Report per task, against the previous phase:

- calls;
- input and output tokens;
- fixed tokens per turn;
- wall clock;
- success rate.

**Targets:**

| Metric | Target | From | To |
| --- | --- | --- | --- |
| Fixed tokens per turn | −40% | 834 | ≤ 500 |
| Screen tokens | −50% | 250 | ≤ 125 |
| Calls per task | ≥ 2× fewer | 5.9 | ≤ 2.9 |

Success rate must not drop, and there must be no safety regression: the journal fence stays, nothing is replayed, and ask-first rules are unchanged.

---

## 6. Constraints (unchanged)

- No raw ADB exposed to agents, and Android prompts are never bypassed. Accessibility on the benchmark emulator was enabled through the visible Settings dialog, not `settings put`.
- `partial` and `unknown` are never replayed; the journal fence stays.
- Ask before destructive, account, purchase, submission or private-capture actions. Near-miss resolution (G1.6) excludes risky verbs.
- Android 8+ (minSdk 26), and Windows, macOS and Linux release hosts.
- One enrolled device per session. **A15 lands before new features** (Step 1).

## 7. Approvals needed from the owner

1. **This plan**, including the ordering in §5.
2. **Outward-facing steps:**
   - opening the PR for this branch;
   - publishing to npm (`android-use` + platform packages);
   - creating the Homebrew tap repo;
   - adding the Claude Code marketplace manifest (in this repo, or a new repo);
   - closing stale Dependabot PRs #10–#18.
3. **Accepting Chrome's first-run terms** on the benchmark emulator, so Chrome tasks can be measured. The alternative is a harness-only Chrome command-line flag file, if the image allows it.
4. **Dependency:** `serde_json/preserve_order`, for key-order-preserving config edits (§3.1). Otherwise accept the text-splice fallback.
5. **Gate K.7:** binary size "+3%, excluding the embedded helper" (+5.7% measured).
6. **Tool rename** to `android_read` / `android_act`, with the dotted names accepted as aliases.


---

## 8. Results (2026-10-10, API 36 emulator, 13 tasks × 10 runs)

Real-tokenizer (o200k) counts from `baseline/trace_tokens.py` over `BENCH_TRACE` output of `cargo xtask benchmark-live`. The first table compares the nine tasks both harnesses share.

| Metric | Baseline (§1) | Now | Change | Target |
| --- | --- | --- | --- | --- |
| Fixed tokens per turn (instructions + both tool definitions) | 834 | 396 | **−53%** | −40% ✓ |
| Calls per task (shared nine) | 5.9 | 2.6 | **2.3× fewer** | ≥ 2× ✓ |
| Calls per task (all thirteen) | — | 2.5 | | |
| Result tokens per task | 426 | 142 | −67% | |
| Input tokens per task (results + fixed per call) | 5,356 | 1,186 (shared nine) | **−78%** | |
| Output tokens per task | 117 | 53 | −55% | |
| Success rate | 64% (29/45) | **98% (127/130)** | +34 pts | no drop ✓ |
| `screen` listing on 7 recorded screens | 543 | 347 | −36% | −50% **missed on listings**; −67% on what each action returns |
| `SKILL.md` loaded per session | 548 | 263 | −52% | |
| Cold `au read status` | 150 ms | 70 ms | −53% | |
| Tap p50 / p95 | 686 / 1,654 ms | 695 / 2,164 ms | unchanged | gate: no regression |
| Helper APK | 52,454 B | 39,148 B | −25% | ≤ 60,000 |
| Host binary, no embed / with helper | 871,360 B (1.0.0) | 888,464 / 937,968 B | +2% / +7.6% | ≤ 1,000,000 |

**Not met, and why.**
- Screen listings fell 36%, not 50%: after dropping tags, subtitles, and wrappers, what remains is the app's own text, which must stay verbatim. The saving that matters comes after actions: a diff or `Done.` instead of a full listing.
- Action latency did not improve. The helper's settle (react, then 200 ms quiet) replaced the host's polling, with the same floor; the saving is in round trips and process spawns, not wall time per tap.
- `find_setting_scroll`, `toggle_wifi`, and `settings_search` each missed once in ten: Settings screens that finish loading after the screen goes quiet. Those runs recover on the next read, as the harness's agent does.

**Plan items by status.**

| Item | Status |
| --- | --- |
| A15 fail-closed serials, A16 boot wait, A8 microphone, A11 link schemes (docs now match: http(s), `geo:`, `google.navigation:`), R1–R6, S1 | Done |
| Q1 fake adb/helper harness | Partly: the agent flows run against a scripted fake helper speaking the wire protocol; no fake adb server (resolution is a pure function with tests) |
| Q2 live harness on the shipped grammar; S legacy deletion; `spec.rs` | Done |
| G1.1–G1.4, G1.6, G1.8–G1.10 | Done |
| G1.5 handles, G2.7 set-of-marks | Done (handles by key; marks on attached images) |
| G1.2 (P2) one round trip | Done for settle (inside the plan); resolve stays a separate query, so a tap is observe + resolve + run + read |
| G1.7 (P4) multi-window | Not needed for dialogs and popups: the active window already is the dialog or menu (checked with a dialog and Chrome's menu). The keyboard is not read by design |
| G2.1 token gates, G2.2–G2.6 | Done |
| G2.4 pruning | Partly: wrappers and repeated text were already merged by the compiler; container-vs-control attribution fixed; no further pruning measured to help |
| G3.1, G3.2 | Done. G3.3 terse-reply snippet not shipped: output is 53 tokens a task, so it could not pay for its own input |
| G4.1 latency gates, G4.3 no uiautomator | Done |
| G4.2 (P3) | Smart sockets done (helper and Chrome). Not done: a daemon for CLI calls (the MCP server already persists; a CLI call is now 70 ms) and blob frames (a screenshot is 1–4 slices of 256 KiB; the JSON/base64 overhead is not the bottleneck) |
| G5.1–G5.6 | Done in the repository; nothing published (needs credentials and the owner's go-ahead) |
| G6.1–G6.4 | Done; budgets recorded in the changelog |
| E1–E4 targets, identity, emulator lifecycle, serve lock | Done on `feature/emulator-targets`; verified end to end on a managed API 36 emulator (boot 38 s, restart keeps enrollment, second serve refused, destroy clean) |
| E5 multi-instance | Verified live: two emulators started concurrently got distinct ports and identifiers beside the default emulator |
| E6 snapshots | Done: `au emulator snapshot NAME save\|load TAG`, verified live |
| E7 remote broker | Not built; needs owner approval |
