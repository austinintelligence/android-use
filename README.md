<p align="center"><img src="images/logo.svg" width="88" alt="Android Use logo"></p>

<h1 align="center">android-use</h1>
<p align="center"><strong>Give AI an Android device.</strong></p>
<p align="center">Your agent sees the screen, taps, types, opens apps, and drives Chrome on a real phone or a local emulator, in about two tool calls per task.</p>

<p align="center">
  <a href="docs/guides/install.md"><strong>Install</strong></a> ·
  <a href="docs/agents/quickstart.md">Connect an agent</a> ·
  <a href="docs/guides/emulators.md">Emulators</a> ·
  <a href="docs/README.md">Documentation</a> ·
  <a href="CHANGELOG.md">Changelog</a>
</p>

<p align="center">
  <a href="https://github.com/austinintelligence/android-use/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/austinintelligence/android-use/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://www.npmjs.com/package/android-use"><img alt="npm" src="https://img.shields.io/npm/v/android-use"></a>
  <a href="https://github.com/austinintelligence/android-use/releases"><img alt="GitHub release" src="https://img.shields.io/github/v/release/austinintelligence/android-use"></a>
  <a href="LICENSE"><img alt="MIT License" src="https://img.shields.io/badge/license-MIT-8CF2C1"></a>
</p>

## One call, then only what changed

<table>
<tr>
<td width="38%"><img src="images/demos/emulator-settings.png" alt="Android Settings, Network and internet, with a numbered box over each control" width="100%"></td>
<td>

A real session on an Android emulator, over MCP. The agent sends one command; the result is the screen it led to.

```text
android_act: open app "Settings" then tap "Network & internet"
Done.
Now on screen:
Network & internet
Navigate up
Internet: AndroidWifi
SIMs: T-Mobile
sw Airplane mode=off
Hotspot & tethering: Off
...

android_act: tap "Airplane mode"
Done.
Changed:
~ Internet: Off
~ SIMs: T-Mobile [disabled]
~ sw Airplane mode=on
```

The picture is what the agent gets when a screen has little text: 17 KB, with a numbered box per control, so `tap e4` works.

</td>
</tr>
</table>

Fewer round trips and shorter results are the point. Measured on an API 36 emulator over MCP, 13 tasks run 10 times each, against the same tasks before the changes in 1.1 ([method and numbers](docs/design/measurements.md)):

| | Before | Now |
| --- | --- | --- |
| Tool calls per task | 5.9 | **2.6** (2.3× fewer) |
| Input tokens per task | 5,356 | **1,186** (−78%) |
| Fixed tokens per turn (instructions + tool definitions) | 834 | **396** (−53%) |
| Output tokens per task | 117 | **53** |
| Tasks that succeeded | 64% | **98%** (127 of 130) |

Tap latency is unchanged (p50 about 700 ms). Screen listings are 36% shorter, not the 50% we aimed for; the saving that matters is after an action, where a one-line diff replaces a full listing.

## Install

You need Android 8 or newer with USB debugging, or an emulator. `au` runs on macOS (Apple Silicon and Intel), Linux (x86_64 and arm64), and Windows (x86_64 and arm64).

```sh
npx -y android-use@1.4.0 setup                 # npm; needs Node.js 20.11+
brew install austinintelligence/tap/android-use && au setup   # Homebrew
curl -fsSL https://raw.githubusercontent.com/austinintelligence/android-use/v1.4.0/install/install.sh | ANDROID_USE_VERSION=1.4.0 sh && au setup
```

On Windows, run `install/install.ps1` in PowerShell, or unzip the archive from [Releases](https://github.com/austinintelligence/android-use/releases) and run `.\au.exe setup`. Every download is checked against `SHA256SUMS`, and the archive contains `au` with the Android helper built in.

`setup` installs the helper on the device, registers Android Use with every agent it finds, and finishes with a live read of the screen. Android asks you to allow USB debugging and to turn on **Android-Use** under Settings → Accessibility; Android Use never skips those prompts. [Step by step →](docs/guides/install.md)

## Use Android Use in your agent

Paste this into Claude Code, Codex, Cursor, Gemini CLI, OpenCode, or any coding agent:

```text
Run `npx -y android-use@1.4.0 setup`, follow its prompts, then use android_read and android_act.
```

Or register one agent yourself:

```sh
au install claude      # also: codex, cursor, gemini, vscode, claude-desktop, opencode, all
au install --dry-run   # show what would change; nothing is written
```

Only the `android-use` entry is added or removed; every other setting stays as it was ([client details](docs/agents/clients.md)). The agent gets two tools, each taking one `command` string: `android_read` looks (`screen`, `page`, `notifications`, `help`), and `android_act` does (`tap "Wi-Fi"`, `type "hello" in "Message"`, `open app "Chrome"`). Agents without MCP use the same commands from a shell: `au read screen`, `au act 'tap "Wi-Fi"'`. Coding agents can start with [AGENTS.md](AGENTS.md).

## No phone? Use an emulator

```sh
au emulator doctor           # needs the Android SDK; if it says none, install Android Studio or the command-line tools first
au emulator install          # fetch the missing emulator package and system image (you accept Google's license)
au emulator create demo
au emulator start demo
au --target demo setup       # then turn on Android-Use in the emulator's Accessibility settings
```

Each emulator is its own target with its own enrollment, journal, and serve lock, so an agent can work on one while you use your phone. Snapshots, restarts, and identity checks are covered in [Emulators and targets](docs/guides/emulators.md).

## What it can do

| | Capability | Notes |
| --- | --- | --- |
| 👁 | **See the screen** | Labels, switches, fields, lists, with state (`sw Wi-Fi=on`). A numbered image when an app exposes little text. |
| 👆 | **Use apps** | Tap, type, scroll (even `until "TEXT"`), go back or home, open apps, chain steps with `then`. |
| 🌐 | **Drive Chrome** | Open pages, click, fill, read, screenshot, through Chrome's own debugging socket. |
| 📱 | **Read the device** | Notifications and one-shot location, when you allow them. |
| 📷 | **Capture on request** | Screenshots; camera, microphone, and screen recording with Android's permission. |
| 🖥 | **Test safely** | Managed emulators with snapshots, side by side with a real device. |

[Capability details →](docs/reference/capabilities.md)

## How it works

<img src="images/how-it-works.svg" alt="Agent to Android Use to Android device workflow" width="100%">

`au` on your computer talks to a small helper app on the device over ADB. Every action is checked against the screen it was planned from, journaled so it runs at most once, and bounded in size and time. Android still owns every sensitive prompt, the helper has no internet permission, and `au` exposes no shell. [Security model →](SECURITY.md) · [Architecture →](docs/concepts/architecture.md)

## Documentation

| I want to… | Read |
| --- | --- |
| Get it running | [Install](docs/guides/install.md), [Troubleshooting](docs/guides/troubleshooting.md) |
| Connect Codex, Claude Code, Cursor, … | [Agent quickstart](docs/agents/quickstart.md), [client setup](docs/agents/clients.md) |
| Look up a command | [CLI](docs/reference/cli.md), [agent protocol](docs/reference/agent-protocol.md), [environment](docs/reference/environment.md) |
| Run on emulators | [Emulators and targets](docs/guides/emulators.md) |
| Understand the cost numbers | [Token budget](docs/concepts/token-budget.md) |
| Run it unattended | [Production guidance](docs/guides/production.md), [security model](docs/concepts/security-model.md) |
| Contribute or cut a release | [Development](docs/contributing/development.md), [testing](docs/contributing/testing.md), [releasing](docs/contributing/release.md) |

## FAQ

<details><summary><strong>Does it include an AI model?</strong></summary><br>No. It gives the agent you already use a compact, safe Android interface.</details>

<details><summary><strong>Does it need screenshots?</strong></summary><br>Usually not. The text screen is smaller and names every actionable control. When a screen has little text, or a label is not found, a compressed screenshot with numbered boxes is attached, and point taps on it map back to exact device pixels.</details>

<details><summary><strong>Does the phone need to be rooted?</strong></summary><br>No. It uses USB debugging and Android's accessibility service, both granted by you.</details>

<details><summary><strong>Can it control any connected device?</strong></summary><br>No. A session is bound to one enrolled device (or one emulator target), checked on every connection.</details>

<details><summary><strong>Wi-Fi instead of USB?</strong></summary><br>Release testing uses USB. Wireless ADB works as an advanced transport; secure it like physical access.</details>

<details><summary><strong>Where do captures and logs go?</strong></summary><br>Large captures are private local artifacts. The journal stores bounded operation metadata, never screen or media contents. <code>au uninstall</code> removes both. See <a href="SECURITY.md">Security</a>.</details>

## Status

Android Use targets Android 8 and newer; the automated checks run on an Android 16 (API 36) emulator and real devices. Optional capabilities vary by device, and `au doctor` is the authority for what a connected one supports. MIT licensed; contributions are welcome, see [CONTRIBUTING.md](CONTRIBUTING.md).
