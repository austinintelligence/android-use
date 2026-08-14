<p align="center"><img src="images/logo.svg" width="88" alt="Android Use logo"></p>

<h1 align="center">android-use</h1>
<p align="center"><strong>Give AI an Android device.</strong></p>
<p align="center">Plug in a phone or tablet. Your AI can see the screen, tap, type, open apps, and use Chrome on the real device.</p>

<p align="center">
  <a href="docs/getting-started.md"><strong>Get started</strong></a> ·
  <a href="docs/README.md">Documentation</a> ·
  <a href="examples/README.md">Examples</a> ·
  <a href="docs/agents/quickstart.md">Agent setup</a> ·
  <a href="docs/reference/cli.md">CLI reference</a>
</p>

<p align="center">
  <a href="https://github.com/austinintelligence/android-use/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/austinintelligence/android-use/actions/workflows/ci.yml/badge.svg"></a>
  <a href="LICENSE"><img alt="MIT License" src="https://img.shields.io/badge/license-MIT-8CF2C1"></a>
  <a href="https://github.com/austinintelligence/android-use/releases"><img alt="GitHub release" src="https://img.shields.io/github/v/release/austinintelligence/android-use?include_prereleases"></a>
</p>

## Your AI can use a real Android device

Android Use connects the AI agent on your computer to the Android device in your hand. Ask normally:

- “Open Settings and check the battery.”
- “Search for Yellowstone in Chrome and show me what you find.”
- “Open this app and go to the account screen.”
- “Tell me which Wi-Fi network is connected.”

The agent reads the same labels and controls you see, then taps, types, scrolls, and checks its work.

<img src="images/demos/yellowstone-browser.png" alt="A real Android Chrome page captured after Android Use navigated to Yellowstone National Park" width="100%">

> **Real device, real result.** Android Use opened this National Park Service page in Chrome on a connected Android tablet and confirmed that Yellowstone loaded.

```text
You: Open Chrome, find Yellowstone, and show me the page.

Agent: Opened Chrome, loaded Yellowstone, and confirmed the page.
```

## What it lets an agent do

| | Capability | What that means |
| --- | --- | --- |
| 👁 | **See what is on screen** | Understand visible text, buttons, fields, lists, and menus. |
| 👆 | **Use your apps** | Tap, type, scroll, go back, return home, and open apps. |
| 🌐 | **Browse with Chrome** | Open pages, follow links, fill fields, read results, and take page screenshots. |
| 📱 | **Check the device** | Read supported device information, location, and notifications when you allow it. |
| 📷 | **Capture when you ask** | Take screenshots or use supported camera, microphone, and screen recording with Android's permission. |
| 🔌 | **Work with your agent** | Connect Codex, Claude Code, Cursor, or another MCP-compatible agent. |

[See the verified capability details →](docs/capabilities.md)

## Setup is one guided command

You need Android 8 or newer, a data-capable USB cable, and Windows, macOS, or Linux.

1. Download the package for your computer from [Releases](https://github.com/austinintelligence/android-use/releases) and unzip it.
2. Connect and unlock your Android device.
3. Run one command:

```console
au setup
```

Android Use checks the cable, remembers the device, installs its small helper, and opens the right Android permission screen. Approve the prompts on the device and leave the command running. It finishes when everything is ready.

```text
Android Use is ready
```

> **Current availability:** v3 packages have not been published yet. The setup above is the finished release flow; developers can use [the source build](docs/development.md) today. The README will not pretend an unpublished npm package exists.

[Full human quickstart →](docs/getting-started.md) · [Troubleshooting →](docs/troubleshooting.md)

## Connect an AI agent

If your client supports MCP, configure it to run:

```console
au serve --mcp
```

The agent receives two focused tools:

- `android.read` sees what is happening.
- `android.act` performs the next small action.

Coding agents can start with [AGENTS.md](AGENTS.md). It contains the whole operating loop and recovery rules in one page. For Codex, Claude Code, Cursor, and similar clients, see the [Agent Quickstart](docs/agents/quickstart.md).

## How it works

<img src="images/how-it-works.svg" alt="Agent to Android Use to Android device workflow" width="100%">

Normal control stays between your computer and the connected device. Android still owns every sensitive permission prompt, and the helper cannot use the internet on its own. [Read the security model →](SECURITY.md)

## Choose your path

| I want to… | Start here |
| --- | --- |
| Let Codex, Claude Code, or Cursor use my Android device | [Agent Quickstart](docs/agents/quickstart.md) |
| Try commands myself | [Human Quickstart](docs/getting-started.md) |
| Build an agent integration | [Agent protocol](docs/reference/agent-protocol.md) |
| Automate a repeatable task | [Examples](examples/README.md) and JSONL |
| Understand permissions and remote-control risk | [Security](SECURITY.md) |
| Contribute to Android Use | [Development](docs/development.md) |

## FAQ

<details><summary><strong>Does Android Use include an AI model?</strong></summary><br>No. It gives an agent a safe, compact Android interface. Bring the agent or application you already use.</details>

<details><summary><strong>Does it work without screenshots?</strong></summary><br>Usually. Semantic UI is the preferred first read because it is smaller and identifies actionable controls. Use a screenshot when visual layout or imagery matters.</details>

<details><summary><strong>Does the phone need to be rooted?</strong></summary><br>No. Android Use relies on Android's normal debugging and user-granted accessibility or capability permissions.</details>

<details><summary><strong>Can it control any connected phone?</strong></summary><br>No. A server session is bound to one enrolled hardware identity. Android also asks you to trust the computer and separately controls sensitive permissions.</details>

<details><summary><strong>Can I use Wi-Fi instead of USB?</strong></summary><br>The current product enrolls an ADB endpoint, but release onboarding and validation use USB. Treat wireless ADB as an advanced Android transport and secure it as carefully as physical debugging access.</details>

<details><summary><strong>Where do captures and logs go?</strong></summary><br>Large captures are stored as local private artifacts. The operation journal stores bounded operation metadata, not screenshot or media contents. See <a href="SECURITY.md">Security</a> for paths, retention, and removal.</details>

## Project status

Android Use is an early open-source release. The typed interface, bounded execution model, helper authentication, Chrome control, and local artifact system are implemented and tested. Platform packages and optional Android capabilities may differ by release and device; `au doctor` is the authority for the connected setup.

MIT licensed. Contributions are welcome—start with [CONTRIBUTING.md](CONTRIBUTING.md).
